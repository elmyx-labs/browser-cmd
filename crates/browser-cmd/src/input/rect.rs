//! An element's box, as the browser's box model reports it.

/// An element's box in viewport coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// The box around one quad of a `DOM.getBoxModel` reply.
    pub(crate) fn around(quad: &serde_json::Value) -> Option<Self> {
        let quad: Vec<f64> =
            quad.as_array().into_iter().flatten().filter_map(serde_json::Value::as_f64).collect();
        if quad.len() < 8 {
            return None;
        }
        let xs = [quad[0], quad[2], quad[4], quad[6]];
        let ys = [quad[1], quad[3], quad[5], quad[7]];
        let left = xs.iter().copied().fold(f64::MAX, f64::min);
        let top = ys.iter().copied().fold(f64::MAX, f64::min);
        let right = xs.iter().copied().fold(f64::MIN, f64::max);
        let bottom = ys.iter().copied().fold(f64::MIN, f64::max);
        Some(Self { left, top, width: right - left, height: bottom - top })
    }

    pub(crate) fn centre(&self) -> (f64, f64) {
        (self.left + self.width / 2.0, self.top + self.height / 2.0)
    }

    pub(crate) fn visible(self, width: f64, height: f64) -> Option<Self> {
        let left = self.left.max(0.0);
        let top = self.top.max(0.0);
        let right = (self.left + self.width).min(width);
        let bottom = (self.top + self.height).min(height);
        (right > left && bottom > top).then_some(Self {
            left,
            top,
            width: right - left,
            height: bottom - top,
        })
    }
}
