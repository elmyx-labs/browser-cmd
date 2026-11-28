//! An element's box in viewport coordinates.

/// An element's box in viewport coordinates.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Rect {
    pub left: f64,
    pub top: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
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
