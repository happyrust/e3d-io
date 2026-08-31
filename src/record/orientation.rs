//! Orientation angles: the three degrees that place an element's Y and Z axes.
//!
//! E3D prints an orientation as two directions — `Y is W 10 S and Z is N 10 W`
//! — while the database stores three angles. Comparing the stored triple
//! against that sentence is not a formatting problem but a geometric one, so
//! this module turns the angles into the two unit vectors and leaves the
//! wording to whoever is reading it.
//!
//! Axes are east, north and up. The three angles, recovered from the eleven
//! orientations in the TTY sample, are applied in order:
//!
//! 1. about east, tilting north towards up;
//! 2. about north, tilting up towards east;
//! 3. about up, turning north towards west.

/// Unit vectors, in (east, north, up), of the element's Y and Z axes.
pub fn axes(angles: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let north = [0.0, 1.0, 0.0];
    let up = [0.0, 0.0, 1.0];
    (rotate(north, angles), rotate(up, angles))
}

fn rotate(mut v: [f64; 3], angles: [f64; 3]) -> [f64; 3] {
    let [about_east, about_north, about_up] = angles.map(f64::to_radians);

    // About east: north towards up.
    v = [
        v[0],
        v[1] * about_east.cos() - v[2] * about_east.sin(),
        v[1] * about_east.sin() + v[2] * about_east.cos(),
    ];
    // About north: up towards east.
    v = [
        v[0] * about_north.cos() + v[2] * about_north.sin(),
        v[1],
        -v[0] * about_north.sin() + v[2] * about_north.cos(),
    ];
    // About up: north towards west.
    [
        v[0] * about_up.cos() - v[1] * about_up.sin(),
        v[0] * about_up.sin() + v[1] * about_up.cos(),
        v[2],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(actual: [f64; 3], expected: [f64; 3]) -> bool {
        actual
            .iter()
            .zip(expected)
            .all(|(a, b)| (a - b).abs() < 1e-6)
    }

    fn bearing_and_rise(v: [f64; 3]) -> (f64, f64) {
        // Bearing measured from north towards west, and the angle above the
        // horizon — the two numbers E3D's wording is built from.
        let bearing = (-v[0]).atan2(v[1]).to_degrees();
        let rise = v[2].asin().to_degrees();
        (bearing, rise)
    }

    #[test]
    fn unrotated_axes_point_north_and_up() {
        let (y, z) = axes([0.0, 0.0, 0.0]);
        assert!(close(y, [0.0, 1.0, 0.0]), "{y:?}");
        assert!(close(z, [0.0, 0.0, 1.0]), "{z:?}");
    }

    /// `[0, 0, 32]` prints as `Y is N 32 W and Z is U`.
    #[test]
    fn third_angle_turns_y_from_north_towards_west() {
        let (y, z) = axes([0.0, 0.0, 32.0]);
        let (bearing, rise) = bearing_and_rise(y);
        assert!((bearing - 32.0).abs() < 1e-6, "bearing {bearing}");
        assert!(rise.abs() < 1e-6);
        assert!(close(z, [0.0, 0.0, 1.0]), "{z:?}");
    }

    /// `[0, 0, -22.5209]` prints as `Y is N 22.521 E and Z is U`: a negative
    /// third angle turns towards east instead.
    #[test]
    fn negative_third_angle_turns_towards_east() {
        let (y, _) = axes([0.0, 0.0, -22.5209]);
        let (bearing, _) = bearing_and_rise(y);
        assert!((bearing + 22.5209).abs() < 1e-6, "bearing {bearing}");
    }

    /// `[0.17651, 0, 0]` prints as `Y is N 0.1765 U and Z is S 89.823 U`.
    #[test]
    fn first_angle_lifts_y_above_the_horizon() {
        let (y, z) = axes([0.17651, 0.0, 0.0]);
        let (y_bearing, y_rise) = bearing_and_rise(y);
        assert!(y_bearing.abs() < 1e-6, "y bearing {y_bearing}");
        assert!((y_rise - 0.17651).abs() < 1e-6, "y rise {y_rise}");

        let (z_bearing, z_rise) = bearing_and_rise(z);
        assert!(
            (z_bearing.abs() - 180.0).abs() < 1e-6,
            "z bearing {z_bearing}"
        );
        assert!((z_rise - (90.0 - 0.17651)).abs() < 1e-4, "z rise {z_rise}");
    }

    /// `[0, 90, 100]` prints as `Y is W 10 S and Z is N 10 W`: the second
    /// angle lays Z into the horizontal plane, the third turns both.
    #[test]
    fn second_angle_rolls_z_into_the_horizontal_plane() {
        let (y, z) = axes([0.0, 90.0, 100.0]);
        let (y_bearing, y_rise) = bearing_and_rise(y);
        assert!((y_bearing - 100.0).abs() < 1e-6, "y bearing {y_bearing}");
        assert!(y_rise.abs() < 1e-6);

        let (z_bearing, z_rise) = bearing_and_rise(z);
        assert!((z_bearing - 10.0).abs() < 1e-6, "z bearing {z_bearing}");
        assert!(z_rise.abs() < 1e-6, "z rise {z_rise}");
    }

    /// `[180, 0, 180]` prints as `Y is N and Z is D`: the first angle turns Y
    /// south and Z down, the third brings Y back to north.
    #[test]
    fn half_turns_about_east_and_up_leave_y_north_and_z_down() {
        let (y, z) = axes([180.0, 0.0, 180.0]);
        assert!(close(y, [0.0, 1.0, 0.0]), "{y:?}");
        assert!(close(z, [0.0, 0.0, -1.0]), "{z:?}");
    }
}
