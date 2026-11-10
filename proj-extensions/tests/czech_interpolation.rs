//! Isolated grid conformance, deliberately excluding Helmert and map projection.
//! Run explicitly with PROJ_CZECH_DATA pointing at Navtron's test_data directory.
#![cfg(feature = "tiff")]

use proj_extensions::{register_cubic_lagrange, CUBIC_LAGRANGE};
use proj_sys::*;
use std::ffi::CString;
use std::path::PathBuf;

fn data_directory() -> PathBuf {
    PathBuf::from(std::env::var_os("PROJ_CZECH_DATA").expect(
        "Set PROJ_CZECH_DATA to the app's test_data directory; run prepare_interpolation_data.py first",
    ))
}

fn number(value: &str) -> f64 {
    let number: f64 = value.parse().unwrap();
    assert!(number.is_finite());
    number
}

fn millimetres(value: f64) -> i64 {
    assert!(value.is_finite());
    (value * 1000.0).round() as i64
}

// The derived CSVs have a deliberately fixed numeric schema, no quoted fields.
// Check the exact header, row widths and counts before using their values.
#[test]
#[ignore = "requires the preserved Czech authority fixtures; see DEVELOPMENT.md"]
fn czech_height_grid_matches_official_rounding() {
    register_cubic_lagrange().unwrap();
    register_cubic_lagrange().unwrap(); // idempotent process-wide setup
    let data = data_directory();
    let csv = std::fs::read_to_string(data.join("czechia/interpolation_height.csv")).unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "id,latitude_deg,longitude_deg,ellipsoidal_height_m,bpv_height_m"
    );
    unsafe {
        let context = proj_context_create();
        assert!(!context.is_null());
        let file = CString::new(data.join("grids/cz_cuzk_CR-2005.tif").to_str().unwrap()).unwrap();
        let grid = proj_ext_grid_open(context, file.as_ptr());
        assert!(!grid.is_null());
        let mut count = 0;
        let mut mismatches = 0;
        let mut max_error_mm = 0.0_f64;
        for line in lines {
            let row: Vec<_> = line.split(',').collect();
            assert_eq!(row.len(), 5);
            let mut undulation = f64::NAN;
            // CR-2005 is sampled at receiver longitude/latitude in radians.
            let status = proj_ext_grid_sample(
                context,
                grid,
                1,
                std::ptr::null(),
                number(row[2]).to_radians(),
                number(row[1]).to_radians(),
                CUBIC_LAGRANGE.as_ptr(),
                1,
                &0,
                &mut undulation,
            );
            assert_eq!(status, 0, "{}: height-grid sampling failed", row[0]);
            let actual = number(row[3]) - undulation;
            let expected = number(row[4]);
            max_error_mm = max_error_mm.max((actual - expected).abs() * 1000.0);
            if millimetres(actual) != millimetres(expected) {
                mismatches += 1;
                eprintln!("{}: height {actual:.9}, expected {expected:.3}", row[0]);
            }
            count += 1;
        }
        proj_ext_grid_close(grid);
        proj_context_destroy(context);
        println!(
            "Height: {count} points, maximum {max_error_mm:.9} mm, rounded mismatches {mismatches}"
        );
        assert_eq!(count, 16);
        assert_eq!(mismatches, 0, "official rounded height mismatch count");
    }
}

#[test]
#[ignore = "requires the preserved Czech authority fixtures; see DEVELOPMENT.md"]
fn czech_horizontal_grid_matches_official_rounding_and_inverse() {
    register_cubic_lagrange().unwrap();
    let data = data_directory();
    let csv = std::fs::read_to_string(data.join("czechia/interpolation_horizontal.csv")).unwrap();
    let mut lines = csv.lines();
    assert_eq!(
        lines.next().unwrap(),
        "id,easting_m,northing_m,height_m,target_easting_m,target_northing_m"
    );
    unsafe {
        let context = proj_context_create();
        assert!(!context.is_null());
        let directory = CString::new(data.join("grids").to_str().unwrap()).unwrap();
        proj_context_set_search_paths(context, 1, &directory.as_ptr());
        let definition = CString::new(format!(
            "+proj=gridshift +grids=cz_cuzk_table_-y-x_3_v1710.tif +interpolation={}",
            CUBIC_LAGRANGE.to_str().unwrap()
        ))
        .unwrap();
        let operation = proj_create(context, definition.as_ptr());
        assert!(!operation.is_null());
        let mut count = 0;
        let mut mismatches = [0; 2];
        let mut max_horizontal_mm = 0.0_f64;
        let mut max_closure_m = 0.0_f64;
        for line in lines {
            let row: Vec<_> = line.split(',').collect();
            assert_eq!(row.len(), 6);
            let input = proj_coord(number(row[1]), number(row[2]), number(row[3]), 2026.0);
            let expected = [number(row[4]), number(row[5])];
            // This direction is S-JTSK -> S-JTSK/05. PROJ applies the TIFF's
            // constant -5,000,000 m offsets; the test must not add them again.
            let actual = proj_trans(operation, PJ_DIRECTION_PJ_FWD, input);
            assert_eq!(proj_errno(operation), 0, "{}: forward failed", row[0]);
            max_horizontal_mm = max_horizontal_mm.max(
                ((actual.v[0] - expected[0]).powi(2) + (actual.v[1] - expected[1]).powi(2)).sqrt()
                    * 1000.0,
            );
            for axis in 0..2 {
                if millimetres(actual.v[axis]) != millimetres(expected[axis]) {
                    mismatches[axis] += 1;
                    eprintln!(
                        "{} axis {axis}: {:.9}, expected {:.3}",
                        row[0], actual.v[axis], expected[axis]
                    );
                }
            }
            assert_eq!([actual.v[2], actual.v[3]], [input.v[2], input.v[3]]);
            let recovered = proj_trans(operation, PJ_DIRECTION_PJ_INV, actual);
            assert_eq!(proj_errno(operation), 0, "{}: inverse failed", row[0]);
            for axis in 0..2 {
                assert!(recovered.v[axis].is_finite());
                max_closure_m = max_closure_m.max((recovered.v[axis] - input.v[axis]).abs());
            }
            // Also close from an independently supplied (rounded) target.
            let target = proj_coord(expected[0], expected[1], input.v[2], input.v[3]);
            let inverse = proj_trans(operation, PJ_DIRECTION_PJ_INV, target);
            let closed = proj_trans(operation, PJ_DIRECTION_PJ_FWD, inverse);
            assert_eq!(
                proj_errno(operation),
                0,
                "{}: target closure failed",
                row[0]
            );
            for axis in 0..2 {
                assert!(closed.v[axis].is_finite());
                max_closure_m = max_closure_m.max((closed.v[axis] - target.v[axis]).abs());
            }
            count += 1;
        }
        proj_destroy(operation);
        proj_context_destroy(context);
        println!("Horizontal: {count} points, maximum {max_horizontal_mm:.9} mm, rounded E/N mismatches {mismatches:?}, closure {:.3} nm", max_closure_m * 1e9);
        assert_eq!(count, 16);
        assert_eq!(
            mismatches,
            [0, 0],
            "official rounded horizontal mismatch counts"
        );
        assert!(
            max_closure_m < 5e-8,
            "same 50 nm closure limit as the Czech pipeline probes"
        );
    }
}
