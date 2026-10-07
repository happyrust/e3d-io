//! Geometry: segment transforms and the circular-arc spans of conic chains.
//!
//! **Transforms.** A segment header may carry six reals `(a, b, c, d, tx,
//! ty)`; a point of the segment is placed by `x' = a·x + c·y + tx`,
//! `y' = b·x + d·y + ty`. A child's transform applies first, then its
//! parent's, up to the picture root whose frame is the sheet in mm. In the
//! Plant samples a `VIEW` segment carries the view's translation
//! (`VIEW.XYPS`) and its `DESI` child the scale (`VIEW.VSCA`), so the design
//! mm below become sheet mm above.
//!
//! **Arcs.** A conic chain is a start point and spans `[mx, my, w, ex, ey]`:
//! from the current point `S` to `E`, `(mx, my)` is the point of the arc over
//! the chord's midpoint and `w = bulge²`, bulge = tan(θ/4) = |M − mid| / half
//! chord (`docs/evidence/2026-09-28-gw-picture-format.md` §3.2). In the frame
//! where `S′ = (−1, 0)`, `E′ = (1, 0)`, `M′ = (0, √w)` the centre is `(0, m)`,
//! `m = (w − 1) / (2√w)`, and the arc always runs from `S′` through `M′` to
//! `E′` **clockwise**, sweeping θ = 4·atan(√w) (w = 1 → 180°, 0.1716 → 90°,
//! 5.83 → 270°); mapping back is `c + x′·u + (y′/√w)·v` with `u` the half
//! chord and `v` = chord midpoint → M. The same formula draws the affine
//! halves of an ellipse (`GOPELL`: w = 1, M the co-vertex). The previous
//! reading (counter-clockwise unwrap) drew every non-semicircle as a
//! 270°–630° loop — round 9 fixed it, `gw\gwpic.py conic_points`.

use super::prim::Span;

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

/// How one span's stored numbers agree with its geometry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcCheck {
    /// Half the chord `|E − S| / 2`.
    pub half_chord: f64,
    /// `|M − chord midpoint|`.
    pub offset: f64,
    /// `offset / half_chord`, the bulge the geometry implies.
    pub bulge_from_geometry: f64,
    /// `√w`, the bulge the record states.
    pub bulge_from_w: f64,
    /// `(M − mid) · (E − S) / 2`: zero when M is on the perpendicular bisector.
    pub dot: f64,
}

impl ArcCheck {
    /// True when M lies on the chord's perpendicular bisector, to
    /// `1e-6 · max(1, half · offset)` — `gw\verify_conic.py`.
    pub fn mid_on_bisector(&self) -> bool {
        self.dot.abs() <= 1e-6 * f64::max(1.0, self.half_chord * self.offset)
    }

    /// True when the geometric bulge equals `√w`, to `1e-6 · max(1, √w)`.
    pub fn bulge_matches(&self) -> bool {
        (self.bulge_from_geometry - self.bulge_from_w).abs()
            <= 1e-6 * f64::max(1.0, self.bulge_from_w)
    }

    /// Both checks.
    pub fn is_consistent(&self) -> bool {
        self.mid_on_bisector() && self.bulge_matches()
    }
}

impl Span {
    /// The swept angle θ = 4·atan(√w) in radians.
    pub fn sweep(&self) -> f64 {
        4.0 * self.w.abs().sqrt().atan()
    }

    /// Compare the span's `w` and `M` with the chord from `start`
    /// (`gw\verify_conic.py`); `None` when the chord is degenerate
    /// (half chord ≤ 1e-9).
    pub fn check(&self, start: (f64, f64)) -> Option<ArcCheck> {
        let (sx, sy) = start;
        let (ex, ey) = self.end;
        let (hx, hy) = ((ex - sx) / 2.0, (ey - sy) / 2.0);
        let (cx, cy) = ((sx + ex) / 2.0, (sy + ey) / 2.0);
        let half_chord = hx.hypot(hy);
        if half_chord <= 1e-9 {
            return None;
        }
        let (vx, vy) = (self.mid.0 - cx, self.mid.1 - cy);
        let offset = vx.hypot(vy);
        Some(ArcCheck {
            half_chord,
            offset,
            bulge_from_geometry: offset / half_chord,
            bulge_from_w: self.w.sqrt(),
            dot: vx * hx + vy * hy,
        })
    }

    /// `segments + 1` points along the arc from `start` to `end`, the first
    /// being `start` and the last `end` exactly (`gw\gwpic.py conic_points`).
    /// A degenerate span (w ≤ 0 or zero chord) yields just the two ends.
    pub fn points(&self, start: (f64, f64), segments: usize) -> Vec<(f64, f64)> {
        let (sx, sy) = start;
        let (ex, ey) = self.end;
        let (cx, cy) = ((sx + ex) / 2.0, (sy + ey) / 2.0);
        let (ux, uy) = ((ex - sx) / 2.0, (ey - sy) / 2.0);
        let (vx, vy) = (self.mid.0 - cx, self.mid.1 - cy);
        let k = if self.w > 0.0 {
            self.w.abs().sqrt()
        } else {
            0.0
        };
        if k < 1e-9 || (ux.abs() < 1e-12 && uy.abs() < 1e-12) || segments == 0 {
            return vec![start, self.end];
        }
        // Normalised frame: S′ = (−1, 0), E′ = (1, 0), M′ = (0, k); centre
        // (0, m), radius R; sweep clockwise from S′ by θ = 4·atan(k).
        let m = (k * k - 1.0) / (2.0 * k);
        let r = (1.0 + m * m).sqrt();
        let a0 = (-m).atan2(-1.0);
        let theta = 4.0 * k.atan();
        let mut points = Vec::with_capacity(segments + 1);
        for i in 0..=segments {
            let t = a0 - theta * i as f64 / segments as f64;
            let xp = r * t.cos();
            let yp = r * t.sin() + m;
            points.push((cx + xp * ux + (yp / k) * vx, cy + xp * uy + (yp / k) * vy));
        }
        points[0] = start;
        points[segments] = self.end;
        points
    }
}

/// Tallies of [`Span::check`] over a set of primitives
/// (`gw\verify_conic.py`'s counters).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArcStats {
    /// Spans with a usable chord.
    pub spans: usize,
    /// Spans with `w = 0` (straight; `M` carries no information).
    pub straight: usize,
    /// Spans whose midpoint is off the perpendicular bisector.
    pub off_bisector: usize,
    /// Spans with `w > 0` whose geometric bulge is not `√w`.
    pub bulge_mismatch: usize,
    /// Spans with `w = 0` whose `M` is nevertheless off the chord.
    pub straight_with_offset: usize,
    /// Spans with a degenerate chord (skipped).
    pub degenerate: usize,
}

impl ArcStats {
    /// Tally every span of `prims`' conic payloads.
    pub fn of<'a>(prims: impl IntoIterator<Item = &'a super::prim::Primitive>) -> Self {
        let mut stats = ArcStats::default();
        for prim in prims {
            let super::prim::Payload::Conic { start, spans } = &prim.payload else {
                continue;
            };
            let mut from = *start;
            for span in spans {
                match span.check(from) {
                    None => stats.degenerate += 1,
                    Some(check) => {
                        stats.spans += 1;
                        if !check.mid_on_bisector() {
                            stats.off_bisector += 1;
                        }
                        if span.w == 0.0 {
                            stats.straight += 1;
                            if !check.bulge_matches() {
                                stats.straight_with_offset += 1;
                            }
                        } else if !check.bulge_matches() {
                            stats.bulge_mismatch += 1;
                        }
                    }
                }
                from = span.end;
            }
        }
        stats
    }

    /// Fold `other` into `self`.
    pub fn add(&mut self, other: &ArcStats) {
        self.spans += other.spans;
        self.straight += other.straight;
        self.off_bisector += other.off_bisector;
        self.bulge_mismatch += other.bulge_mismatch;
        self.straight_with_offset += other.straight_with_offset;
        self.degenerate += other.degenerate;
    }

    /// True when every curved span's `M` and `w` agree with its chord.
    pub fn curved_all_consistent(&self) -> bool {
        self.off_bisector == 0 && self.bulge_mismatch == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn a_semicircle_sweeps_180_degrees_clockwise_through_its_midpoint() {
        // Unit circle, start (−1, 0) → end (1, 0) through M = (0, 1): w = 1.
        let span = Span {
            mid: (0.0, 1.0),
            w: 1.0,
            end: (1.0, 0.0),
        };
        let start = (-1.0, 0.0);
        assert!((span.sweep() - std::f64::consts::PI).abs() < 1e-12);
        let pts = span.points(start, 4);
        assert_eq!(pts.len(), 5);
        assert_eq!(pts[0], start);
        assert_eq!(pts[4], span.end);
        assert!(close(pts[2], (0.0, 1.0)), "{:?}", pts[2]);
        // Clockwise: x grows from −1 to 1 while y stays ≥ 0.
        let d = std::f64::consts::FRAC_1_SQRT_2;
        assert!(close(pts[1], (-d, d)), "{:?}", pts[1]);
        assert!(close(pts[3], (d, d)), "{:?}", pts[3]);
        let check = span.check(start).unwrap();
        assert!(check.is_consistent(), "{check:?}");
        assert!((check.bulge_from_geometry - 1.0).abs() < 1e-12);
    }

    #[test]
    fn an_elbow_is_a_quarter_circle_and_a_bulge_over_one_a_long_way_round() {
        // 90° elbow: S = (1, 0), E = (0, 1) on the unit circle about the
        // origin; M = (√½, √½); bulge = tan(22.5°) → w = 0.1716.
        let d = std::f64::consts::FRAC_1_SQRT_2;
        let k = (std::f64::consts::FRAC_PI_8).tan();
        let span = Span {
            mid: (d, d),
            w: k * k,
            end: (0.0, 1.0),
        };
        let start = (1.0, 0.0);
        assert!((span.sweep() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        let pts = span.points(start, 2);
        assert!(close(pts[1], (d, d)), "{:?}", pts[1]);
        for p in span.points(start, 8) {
            assert!(
                (p.0.hypot(p.1) - 1.0).abs() < 1e-9,
                "{p:?} is off the circle"
            );
            assert!(p.0 >= -1e-9 && p.1 >= -1e-9, "{p:?} left the quadrant");
        }
        assert!(span.check(start).unwrap().is_consistent());

        // 270° the other way round: same chord, M = (−√½, −√½), w = tan(67.5°)².
        let k = (3.0 * std::f64::consts::FRAC_PI_8).tan();
        let long = Span {
            mid: (-d, -d),
            w: k * k,
            end: (0.0, 1.0),
        };
        assert!((long.sweep() - 3.0 * std::f64::consts::FRAC_PI_2).abs() < 1e-12);
        assert!((long.w - 5.828427).abs() < 1e-6);
        let pts = long.points(start, 6);
        assert!(close(pts[3], (-d, -d)), "{:?}", pts[3]);
        for p in &pts {
            assert!(
                (p.0.hypot(p.1) - 1.0).abs() < 1e-9,
                "{p:?} is off the circle"
            );
        }
        // The long arc passes through (0, −1) and (−1, 0), the short one does not.
        assert!(pts.iter().any(|p| close(*p, (0.0, -1.0))) || pts.iter().any(|p| p.1 < -0.9));
        assert!(long.check(start).unwrap().is_consistent());
    }

    #[test]
    fn a_span_whose_midpoint_is_off_the_bisector_or_off_w_fails_the_check() {
        let start = (-1.0, 0.0);
        let skewed = Span {
            mid: (0.3, 1.0),
            w: 1.0,
            end: (1.0, 0.0),
        };
        let check = skewed.check(start).unwrap();
        assert!(!check.mid_on_bisector());
        let wrong_w = Span {
            mid: (0.0, 1.0),
            w: 0.5,
            end: (1.0, 0.0),
        };
        assert!(!wrong_w.check(start).unwrap().bulge_matches());
        // A zero chord has no check and expands to its two ends.
        let degenerate = Span {
            mid: (0.0, 1.0),
            w: 1.0,
            end: start,
        };
        assert_eq!(degenerate.check(start), None);
        assert_eq!(degenerate.points(start, 8), vec![start, start]);
    }

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
