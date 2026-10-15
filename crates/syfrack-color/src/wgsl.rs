//! The display shader of the studio must use the very same constants as the
//! CPU pipeline. They are emitted here, never copied by hand.

use crate::encode::SRGB_TO_P3;
use crate::tonemap::{AGX_IN, AGX_OUT, MAX_EV, MIN_EV};

fn mat(name: &str, m: &[[f32; 3]; 3]) -> String {
    // WGSL mat3x3f constructors take COLUMNS; the Rust matrices are row-major (v' = M v).
    let col = |j: usize| format!("vec3f({:.9}, {:.9}, {:.9})", m[0][j], m[1][j], m[2][j]);
    format!(
        "const {name}: mat3x3f = mat3x3f({}, {}, {});\n",
        col(0),
        col(1),
        col(2)
    )
}

/// WGSL constants for the display shader, generated from the CPU pipeline.
pub fn wgsl_constants() -> String {
    let mut s = String::new();
    s.push_str(&mat("AGX_IN", &AGX_IN));
    s.push_str(&mat("AGX_OUT", &AGX_OUT));
    s.push_str(&format!("const AGX_MIN_EV: f32 = {MIN_EV:.9};\n"));
    s.push_str(&format!("const AGX_MAX_EV: f32 = {MAX_EV:.9};\n"));
    s.push_str("const LUMA: vec3f = vec3f(0.212600000, 0.715200000, 0.072200000);\n");
    s.push_str(&mat("SRGB_TO_P3", &SRGB_TO_P3));
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tonemap::{AGX_IN, AGX_OUT, MAX_EV, MIN_EV};

    fn f(x: f32) -> String {
        format!("{x:.9}")
    }

    #[test]
    fn constants_are_emitted_column_major() {
        let s = wgsl_constants();
        // The first vec3f of AGX_IN must be the first COLUMN of the Rust row-major matrix.
        let col0 = format!(
            "vec3f({}, {}, {})",
            f(AGX_IN[0][0]),
            f(AGX_IN[1][0]),
            f(AGX_IN[2][0])
        );
        assert!(s.contains(&col0), "missing {col0} in:\n{s}");
        let out_col2 = format!(
            "vec3f({}, {}, {})",
            f(AGX_OUT[0][2]),
            f(AGX_OUT[1][2]),
            f(AGX_OUT[2][2])
        );
        assert!(s.contains(&out_col2));
        assert!(s.contains(&format!("const AGX_MIN_EV: f32 = {};", f(MIN_EV))));
        assert!(s.contains(&format!("const AGX_MAX_EV: f32 = {};", f(MAX_EV))));
        assert!(s.contains("const LUMA: vec3f = vec3f(0.212600000, 0.715200000, 0.072200000);"));
        assert!(s.contains("const SRGB_TO_P3: mat3x3f"));
        assert_eq!(s.matches("const ").count(), 6);
    }
}
