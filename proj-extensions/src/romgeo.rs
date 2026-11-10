//! CNC ROMGEO v1.0.2.48's bicubic Hermite derivative convention.
//! Reference: cartografie-ro/romgeo-table-convert-gui, romgeo_lite/transformations.py.
//! The basis evaluation below is independent of CNC's expanded polynomial.
//! Nearest sampling uses Python-compatible ties-to-even, without extrapolation.
use super::*;

pub fn register_romgeo_interpolators() -> Result<(), RegistrationError> {
    static STATUS: OnceLock<c_int> = OnceLock::new();
    let code = *STATUS.get_or_init(|| unsafe {
        let context = proj_context_create();
        if context.is_null() {
            return PROJ_ERR_OTHER as c_int;
        }
        let mut code = 0;
        let entries: [(&CStr, PROJ_EXT_INTERPOLATE); 2] = [
            (c"rust_cnc_bicubic", Some(bicubic)),
            (c"rust_nearest_neighbour", Some(nearest)),
        ];
        for (name, callback) in entries {
            let descriptor = PROJ_EXT_INTERPOLATOR {
                abi_version: PROJ_EXT_ABI_VERSION,
                struct_size: std::mem::size_of::<PROJ_EXT_INTERPOLATOR>(),
                interpolate: callback,
            };
            code = proj_ext_register_interpolator(context, name.as_ptr(), &descriptor);
            if code != 0 {
                break;
            }
        }
        proj_context_destroy(context);
        code
    });
    if code == 0 {
        Ok(())
    } else {
        Err(RegistrationError {
            kernel: "ROMGEO interpolation",
            code,
        })
    }
}

// No allocation or panicking operations in callbacks; all borrowed pointers
// remain local. Reader errors propagate before publishing the affected band.
unsafe extern "C" fn nearest(
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
        if info.width < 1
            || info.height < 1
            || x < 0.0
            || y < 0.0
            || x > (info.width - 1) as f64
            || y > (info.height - 1) as f64
        {
            return PROJ_ERR_COORD_TRANSFM_OUTSIDE_GRID as c_int;
        }
        for band in 0..count as usize {
            let mut sample = 0_f32;
            let code = proj_ext_grid_read(
                view,
                x.round_ties_even() as c_int,
                y.round_ties_even() as c_int,
                1,
                1,
                1,
                bands.add(band),
                &mut sample,
                1,
            );
            if code != 0 {
                return code;
            }
            *values.add(band) = f64::from(sample);
        }
        0
    }
}

unsafe extern "C" fn bicubic(
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
            let result = evaluate(samples.map(f64::from), x - column as f64, y - row as f64);
            if !result.is_finite() {
                return PROJ_ERR_COORD_TRANSFM_INVALID_COORD as c_int;
            }
            *values.add(band) = result;
        }
        0
    }
}

// Cubic Hermite basis for endpoint values followed by endpoint derivatives.
fn basis(t: f64) -> [f64; 4] {
    let t2 = t * t;
    let t3 = t2 * t;
    [
        2.0 * t3 - 3.0 * t2 + 1.0,
        -2.0 * t3 + 3.0 * t2,
        t3 - 2.0 * t2 + t,
        t3 - t2,
    ]
}

fn evaluate(s: [f64; 16], x: f64, y: f64) -> f64 {
    // Samples are south-to-north rows, west-to-east columns. The middle
    // four nodes bound the evaluated cell. CNC uses one-sided first
    // derivatives and centered mixed derivatives; this is NOT Catmull-Rom.
    let dx00 = (-s[7] + 4.0 * s[6] - 3.0 * s[5]) / 2.0;
    let dx10 = (3.0 * s[6] - 4.0 * s[5] + s[4]) / 2.0;
    let dx01 = (-s[11] + 4.0 * s[10] - 3.0 * s[9]) / 2.0;
    let dx11 = (3.0 * s[10] - 4.0 * s[9] + s[8]) / 2.0;
    let dy00 = (-s[13] + 4.0 * s[9] - 3.0 * s[5]) / 2.0;
    let dy10 = (-s[14] + 4.0 * s[10] - 3.0 * s[6]) / 2.0;
    let dy01 = (3.0 * s[9] - 4.0 * s[5] + s[1]) / 2.0;
    let dy11 = (3.0 * s[10] - 4.0 * s[6] + s[2]) / 2.0;
    let dxy00 = (s[0] + s[10] - s[2] - s[8]) / 4.0;
    let dxy10 = (s[1] + s[11] - s[3] - s[9]) / 4.0;
    let dxy01 = (s[4] + s[14] - s[6] - s[12]) / 4.0;
    let dxy11 = (s[5] + s[15] - s[7] - s[13]) / 4.0;
    let data = [
        [s[5], s[6], dx00, dx10],
        [s[9], s[10], dx01, dx11],
        [dy00, dy10, dxy00, dxy10],
        [dy01, dy11, dxy01, dxy11],
    ];
    let wx = basis(x);
    let wy = basis(y);
    let mut result = 0.0;
    for j in 0..4 {
        for i in 0..4 {
            result += data[j][i] * wx[i] * wy[j];
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bicubic_matches_cnc_expanded_polynomial() {
        // Independent values from the shipped v1.0.2.48 _doBSInterpolation,
        // using a 6x6 grid with this asymmetric 4x4 stencil at rows/cols 1..4.
        // That implementation uses an expanded polynomial, not our tensor basis.
        let stencil = std::array::from_fn(|i| (1_u32 << i) as f64);
        for (x, y, expected) in [
            (0.0, 0.0, 32.0),
            (0.125, 0.375, -360.3323163986206),
            (0.2, 0.7, 101.15232000000071),
            (0.6, 0.4, -496.18048000000033),
        ] {
            assert!((evaluate(stencil, x, y) - expected).abs() < 1e-10);
        }
        assert_eq!(evaluate(stencil, 1.0, 0.0), 64.0);
        assert_eq!(evaluate(stencil, 0.0, 1.0), 512.0);
        assert_eq!(evaluate(stencil, 1.0, 1.0), 1024.0);
    }

    #[cfg(feature = "tiff")]
    #[test]
    fn sampling_preserves_reader_failures_and_nearest_needs_only_one_node() {
        register_romgeo_interpolators().unwrap();
        register_romgeo_interpolators().unwrap();
        let path = std::ffi::CString::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../proj-sys/PROJSRC/proj/data/tests/test_vgrid_nodata.tif"
        ))
        .unwrap();
        unsafe {
            let context = proj_context_create();
            assert!(!context.is_null());
            let grid = proj_ext_grid_open(context, path.as_ptr());
            assert!(!grid.is_null());
            for (kernel, longitude, expected_error) in [
                (
                    c"rust_cnc_bicubic",
                    4.05_f64,
                    PROJ_ERR_COORD_TRANSFM_OUTSIDE_GRID,
                ),
                (
                    c"rust_cnc_bicubic",
                    4.15,
                    PROJ_ERR_COORD_TRANSFM_GRID_AT_NODATA,
                ),
                (
                    c"rust_nearest_neighbour",
                    4.01,
                    PROJ_ERR_COORD_TRANSFM_GRID_AT_NODATA,
                ),
                (c"rust_nearest_neighbour", 4.11, 0),
            ] {
                let mut value = 123.0;
                let status = proj_ext_grid_sample(
                    context,
                    grid,
                    1,
                    std::ptr::null(),
                    longitude.to_radians(),
                    52.11_f64.to_radians(),
                    kernel.as_ptr(),
                    1,
                    &0,
                    &mut value,
                );
                assert_eq!(status, expected_error as c_int);
                assert_eq!(value, if expected_error == 0 { 10.0 } else { 123.0 });
            }
            proj_ext_grid_close(grid);
            proj_context_destroy(context);
        }
    }
}
