//! Segment transforms: the six reals `(a, b, c, d, tx, ty)` a segment
//! header may carry, and how they compose down the tree.
//!
//! A point of a segment is placed by `x' = a·x + c·y + tx`,
//! `y' = b·x + d·y + ty`; a child's transform applies first, then its
//! parent's, up to the picture root whose frame is the sheet in mm. In the
//! Plant samples a `VIEW` segment carries the view's translation
//! (`VIEW.XYPS`) and its `DESI` child the scale (`VIEW.VSCA`), so the design
//! mm below become sheet mm above.

/// An affine map `(a, b, c, d, tx, ty)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform(pub [f64; 6]);

impl Transform {
    /// The identity.
    pub const IDENTITY: Transform = Transform([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// Place `(x, y)`.
    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        let [a, b, c, d, tx, ty] = self.0;
        (a * x + c * y + tx, b * x + d * y + ty)
    }

    /// `self ∘ inner`: apply `inner` first, then `self`
    /// (`gwdump.py xf_compose(outer, inner)`).
    pub fn then_outer(&self, inner: &Transform) -> Transform {
        let [a1, b1, c1, d1, e1, f1] = inner.0;
        let [a2, b2, c2, d2, e2, f2] = self.0;
        Transform([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }

    /// True for the identity map.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl From<[f64; 6]> for Transform {
    fn from(t: [f64; 6]) -> Self {
        Transform(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_applies_the_inner_map_first() {
        // VIEW translation over DESI scale: a design point at (100, 200) mm
        // lands at 60.98 + 1.06, 177.81 + 2.12 on the sheet.
        let view = Transform([1.0, 0.0, 0.0, 1.0, 60.98, 177.81]);
        let desi = Transform([0.0106, 0.0, 0.0, 0.0106, 0.0, 0.0]);
        let composed = view.then_outer(&desi);
        let (x, y) = composed.apply(100.0, 200.0);
        let (ex, ey) = view.apply(desi.apply(100.0, 200.0).0, desi.apply(100.0, 200.0).1);
        assert!((x - ex).abs() < 1e-12 && (y - ey).abs() < 1e-12);
        assert!((x - 62.04).abs() < 1e-9);
        assert!((y - 179.93).abs() < 1e-9);
        assert!(Transform::IDENTITY.is_identity());
        assert_eq!(Transform::IDENTITY.then_outer(&desi), desi);
        assert_eq!(desi.then_outer(&Transform::IDENTITY), desi);
    }
}
