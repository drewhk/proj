#![cfg(all(feature = "native_extensions", not(feature = "nobuild")))]

use proj_sys::*;
#[cfg(feature = "tiff")]
use std::ffi::CString;
use std::ffi::{c_char, c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

static CREATED: AtomicUsize = AtomicUsize::new(0);
static DESTROYED: AtomicUsize = AtomicUsize::new(0);

// Callbacks deliberately contain no assertions or other panicking operations.
// Rust must never unwind across the native ABI boundary.
unsafe extern "C" fn create(
    _: *mut PJ_CONTEXT,
    _: c_int,
    _: *const *const c_char,
    _: f64,
    _: f64,
    instance: *mut PROJ_EXT_OPERATION_INSTANCE,
) -> c_int {
    unsafe {
        (*instance).state = Box::into_raw(Box::new(2.0_f64)).cast();
        (*instance).input_domain = PROJ_EXT_DOMAIN_PROJ_EXT_PROJECTED as c_int;
        (*instance).output_domain = PROJ_EXT_DOMAIN_PROJ_EXT_PROJECTED as c_int;
    }
    CREATED.fetch_add(1, Ordering::SeqCst);
    0
}

unsafe extern "C" fn forward(
    _: *mut PJ_CONTEXT,
    state: *mut c_void,
    coord: *mut PJ_COORD,
) -> c_int {
    unsafe { (*coord).v[2] *= *state.cast::<f64>() };
    0
}

unsafe extern "C" fn inverse(
    _: *mut PJ_CONTEXT,
    state: *mut c_void,
    coord: *mut PJ_COORD,
) -> c_int {
    unsafe { (*coord).v[2] /= *state.cast::<f64>() };
    0
}

unsafe extern "C" fn destroy(state: *mut c_void) {
    unsafe { drop(Box::from_raw(state.cast::<f64>())) };
    DESTROYED.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn rust_operation_callbacks_link_transform_and_clone() {
    unsafe {
        let ctx = proj_context_create();
        let other_ctx = proj_context_create();
        assert!(!ctx.is_null() && !other_ctx.is_null());
        let callbacks = PROJ_EXT_OPERATION {
            abi_version: PROJ_EXT_ABI_VERSION,
            struct_size: std::mem::size_of::<PROJ_EXT_OPERATION>(),
            create: Some(create),
            forward: Some(forward),
            inverse: Some(inverse),
            destroy: Some(destroy),
            reassign_context: None,
        };
        assert_eq!(
            proj_ext_register_operation(ctx, c"rust_test_height".as_ptr(), &callbacks),
            0
        );
        let op = proj_create(ctx, c"+proj=rust_test_height".as_ptr());
        assert!(!op.is_null());
        let input = proj_coord(10.0, 20.0, 123.0, 2026.0);
        let output = proj_trans(op, PJ_DIRECTION_PJ_FWD, input);
        assert_eq!(output.v, [10.0, 20.0, 246.0, 2026.0]);
        assert_eq!(proj_trans(op, PJ_DIRECTION_PJ_INV, output).v, input.v);

        // The clone owns a newly constructed Rust allocation, independently
        // of both the original operation and its context.
        let clone = proj_clone(other_ctx, op);
        assert!(!clone.is_null());
        // PROJ also constructs and destroys a temporary operation while
        // validating the input string. Count live states, not constructor calls.
        assert_eq!(
            CREATED.load(Ordering::SeqCst) - DESTROYED.load(Ordering::SeqCst),
            2
        );
        proj_destroy(op);
        proj_context_destroy(ctx);
        assert_eq!(proj_trans(clone, PJ_DIRECTION_PJ_FWD, input).v, output.v);
        proj_destroy(clone);
        proj_context_destroy(other_ctx);
        assert_eq!(
            DESTROYED.load(Ordering::SeqCst),
            CREATED.load(Ordering::SeqCst)
        );
    }
}

#[cfg(feature = "tiff")]
unsafe extern "C" fn interpolate(
    _: *mut PJ_CONTEXT,
    view: *const PROJ_EXT_GRID_VIEW,
    x: f64,
    y: f64,
    count: c_int,
    bands: *const c_int,
    values: *mut f64,
) -> c_int {
    if count != 1 {
        return PROJ_ERR_OTHER_API_MISUSE as c_int;
    }
    unsafe {
        let info = &*proj_ext_grid_info(view);
        if info.width < 2
            || info.height < 2
            || x < 0.0
            || y < 0.0
            || x > (info.width - 1) as f64
            || y > (info.height - 1) as f64
        {
            return PROJ_ERR_COORD_TRANSFM_OUTSIDE_GRID as c_int;
        }
        let column = (x.floor() as c_int).min(info.width - 2);
        let row = (y.floor() as c_int).min(info.height - 2);
        let mut samples = [0_f32; 4];
        let status = proj_ext_grid_read(
            view,
            column,
            row,
            2,
            2,
            count,
            bands,
            samples.as_mut_ptr(),
            samples.len(),
        );
        if status != 0 {
            return status;
        }
        let fx = x - column as f64;
        let fy = y - row as f64;
        *values = (1.0 - fx) * (1.0 - fy) * samples[0] as f64
            + fx * (1.0 - fy) * samples[1] as f64
            + (1.0 - fx) * fy * samples[2] as f64
            + fx * fy * samples[3] as f64;
    }
    0
}

#[cfg(feature = "tiff")]
#[test]
fn rust_interpolator_reads_decoded_native_grid_samples() {
    let file = CString::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/PROJSRC/proj/data/tests/test_vgrid_uint16_with_scale_offset.tif"
    ))
    .unwrap();
    unsafe {
        let ctx = proj_context_create();
        assert!(!ctx.is_null());
        let callbacks = PROJ_EXT_INTERPOLATOR {
            abi_version: PROJ_EXT_ABI_VERSION,
            struct_size: std::mem::size_of::<PROJ_EXT_INTERPOLATOR>(),
            interpolate: Some(interpolate),
        };
        assert_eq!(
            proj_ext_register_interpolator(ctx, c"rust_test_bilinear".as_ptr(), &callbacks),
            0
        );
        let grid = proj_ext_grid_open(ctx, file.as_ptr());
        assert!(!grid.is_null());
        let mut value = f64::NAN;
        assert_eq!(
            proj_ext_grid_sample(
                ctx,
                grid,
                1,
                std::ptr::null(),
                4.5_f64.to_radians(),
                52.5_f64.to_radians(),
                c"rust_test_bilinear".as_ptr(),
                1,
                &0,
                &mut value,
            ),
            0
        );
        assert!((value - 11.5).abs() < 1e-12);
        proj_ext_grid_close(grid);
        proj_context_destroy(ctx);
    }
}
