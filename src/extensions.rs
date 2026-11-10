//! Reusable algorithms registered through our native fork's extension ABI.
//! Registration is explicit; importing this module does not change PROJ defaults.

use crate::ProjError;
use proj_sys::*;
use std::ffi::{c_int, CStr};
use std::sync::OnceLock;

mod romgeo;
pub use romgeo::register_romgeo_interpolators;

/// Name used in `+proj=gridshift +interpolation=rust_cubic_lagrange` and raw sampling.
pub const CUBIC_LAGRANGE: &CStr = c"rust_cubic_lagrange";

/// Register tensor-product cubic Lagrange interpolation once per process.
///
/// Uses nodes `floor(index)-1 .. floor(index)+2` on each axis. The complete 4x4
/// window must exist and contain no nodata. There is no fallback, extrapolation
/// or shifted edge stencil. This describes the supported interior domain; it
/// does not claim to reproduce any authority's boundary policy.
/// Grid sampling requires the `tiff` feature. Repeated calls are safe.
pub fn register_cubic_lagrange() -> Result<(), ProjError> {
    static STATUS: OnceLock<c_int> = OnceLock::new();
    let code = *STATUS.get_or_init(|| unsafe {
        // Keep registration errors off the process-wide default context.
        let context = proj_context_create();
        if context.is_null() {
            return PROJ_ERR_OTHER as c_int;
        }
        let descriptor = PROJ_EXT_INTERPOLATOR {
            abi_version: PROJ_EXT_ABI_VERSION,
            struct_size: std::mem::size_of::<PROJ_EXT_INTERPOLATOR>(),
            interpolate: Some(cubic_lagrange),
        };
        let code = proj_ext_register_interpolator(context, CUBIC_LAGRANGE.as_ptr(), &descriptor);
        proj_context_destroy(context);
        code
    });
    if code == 0 {
        Ok(())
    } else {
        Err(ProjError::Projection(format!(
            "Cubic Lagrange registration failed with PROJ error {code}"
        )))
    }
}

// Basis at nodes [-1, 0, 1, 2], evaluated at the fractional cell position.
// Symbolically checked: sum(w[i] * node[i]^k) == t^k for k = 0,1,2,3.
fn weights(t: f64) -> [f64; 4] {
    [
        -t * (t - 1.0) * (t - 2.0) / 6.0,
        (t + 1.0) * (t - 1.0) * (t - 2.0) / 2.0,
        -(t + 1.0) * t * (t - 2.0) / 2.0,
        (t + 1.0) * t * (t - 1.0) / 6.0,
    ]
}

// PROJ lends all pointers for this call only. The callback does not allocate,
// retain pointers or invoke panicking operations: unwinding across C is forbidden.
unsafe extern "C" fn cubic_lagrange(
    _: *mut PJ_CONTEXT,
    view: *const PROJ_EXT_GRID_VIEW,
    x: f64,
    y: f64,
    count: c_int,
    bands: *const c_int,
    values: *mut f64,
) -> c_int {
    unsafe {
        let info = proj_ext_grid_info(view);
        if info.is_null()
            || bands.is_null()
            || values.is_null()
            || count <= 0
            || !x.is_finite()
            || !y.is_finite()
        {
            return PROJ_ERR_OTHER_API_MISUSE as c_int;
        }
        let info = &*info;
        // Check before converting floating indices or subtracting from dimensions.
        // The upper bound is strict because the prescribed stencil starts at floor-1.
        if info.width < 4
            || info.height < 4
            || x < 1.0
            || y < 1.0
            || x >= (info.width - 2) as f64
            || y >= (info.height - 2) as f64
        {
            return PROJ_ERR_COORD_TRANSFM_OUTSIDE_GRID as c_int;
        }
        let column = x.floor() as c_int;
        let row = y.floor() as c_int;
        let wx = weights(x - column as f64);
        let wy = weights(y - row as f64);
        // Read one band at a time: fixed stack storage works for any band count.
        for band in 0..count as usize {
            let mut samples = [0_f32; 16];
            let code = proj_ext_grid_read(
                view,
                column - 1,
                row - 1,
                4,
                4,
                1,
                bands.add(band),
                samples.as_mut_ptr(),
                samples.len(),
            );
            if code != 0 {
                return code;
            }
            let mut result = 0.0;
            // Reader windows run south-to-north, west-to-east, independent of
            // the TIFF's on-disk scan order. No geographic signs/units belong here.
            for (j, y_weight) in wy.iter().enumerate() {
                for (i, x_weight) in wx.iter().enumerate() {
                    result += y_weight * x_weight * f64::from(samples[j * 4 + i]);
                }
            }
            if !result.is_finite() {
                return PROJ_ERR_COORD_TRANSFM_INVALID_COORD as c_int;
            }
            *values.add(band) = result;
        }
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "tiff")]
    #[test]
    fn incomplete_window_and_nodata_are_explicit_errors() {
        register_cubic_lagrange().unwrap();
        let path = std::ffi::CString::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/proj-sys/PROJSRC/proj/data/tests/test_vgrid_nodata.tif"
        ))
        .unwrap();
        unsafe {
            let context = proj_context_create();
            assert!(!context.is_null());
            let grid = proj_ext_grid_open(context, path.as_ptr());
            assert!(!grid.is_null());
            // This fixture has 4x4 nodes at lon 4.0..4.3, lat 52.0..52.3.
            // Both points are inside its extent; only the central cell has a
            // full 4x4 stencil. That stencil contains an explicit nodata node.
            for (longitude, expected_error) in [
                (4.05_f64, PROJ_ERR_COORD_TRANSFM_OUTSIDE_GRID),
                (4.15_f64, PROJ_ERR_COORD_TRANSFM_GRID_AT_NODATA),
            ] {
                let mut output = 123.0;
                let status = proj_ext_grid_sample(
                    context,
                    grid,
                    1,
                    std::ptr::null(),
                    longitude.to_radians(),
                    52.15_f64.to_radians(),
                    CUBIC_LAGRANGE.as_ptr(),
                    1,
                    &0,
                    &mut output,
                );
                assert_eq!(status, expected_error as c_int);
                assert_eq!(
                    output, 123.0,
                    "failed sampling must not publish a partial result"
                );
            }
            proj_ext_grid_close(grid);
            proj_context_destroy(context);
        }
    }

    #[test]
    fn basis_reproduces_cubics_and_tensor_products() {
        for x in [0.0, 0.125, 0.5, 0.875, 1.0] {
            for y in [0.0, 0.25, 0.625, 1.0] {
                let wx = weights(x);
                let wy = weights(y);
                // Mixed powers exercise both axes, rather than just constants
                // or a symmetric polynomial that could conceal an axis swap.
                let polynomial =
                    |a: f64, b: f64| 2.0 * a * a * a - 7.0 * b * b + a * a * b * b * b + 3.0;
                let mut result = 0.0;
                for j in 0..4 {
                    for i in 0..4 {
                        result += wx[i] * wy[j] * polynomial(i as f64 - 1.0, j as f64 - 1.0);
                    }
                }
                assert!((result - polynomial(x, y)).abs() < 1e-12);
            }
        }
    }
}
