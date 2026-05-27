#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Px(pub i32);

impl Px {
    pub const ZERO: Px = Px(0);
    #[inline]
    pub const fn new(n: i32) -> Self {
        Px(n)
    }
    #[inline]
    pub const fn raw(self) -> i32 {
        self.0
    }
    /// CSS px → device px. Only called at the canvas seam.
    #[inline]
    pub fn to_device(self, dpr: f64) -> f64 {
        self.0 as f64 * dpr
    }
}

impl core::ops::Add for Px {
    type Output = Px;
    fn add(self, r: Px) -> Px {
        Px(self.0 + r.0)
    }
}
impl core::ops::Sub for Px {
    type Output = Px;
    fn sub(self, r: Px) -> Px {
        Px(self.0 - r.0)
    }
}
impl core::ops::AddAssign for Px {
    fn add_assign(&mut self, r: Px) {
        self.0 += r.0;
    }
}
impl core::ops::SubAssign for Px {
    fn sub_assign(&mut self, r: Px) {
        self.0 -= r.0;
    }
}

impl Px {
    pub const fn const_add(self, r: Px) -> Px {
        Px(self.0 + r.0)
    }
    pub const fn const_sub(self, r: Px) -> Px {
        Px(self.0 - r.0)
    }
}
