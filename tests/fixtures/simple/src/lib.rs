//! Simple fixture for end-to-end tests.

/// Adds two numbers.
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

/// Calls add.
pub fn call_add() -> i32 {
    add(2, 3)
}

/// A point in 2D space.
pub struct Point {
    pub x: i32,
    pub y: i32,
}

impl Point {
    /// The origin.
    pub fn origin() -> Self {
        Self { x: 0, y: 0 }
    }
}

/// Something that can describe itself.
pub trait Describe {
    fn describe(&self) -> String;
}

impl Describe for Point {
    fn describe(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

/// Builds a point and uses associated items.
pub fn make_point() -> Point {
    let p = Point { x: 1, y: 2 };
    let origin = Point::origin();
    let _ = (p, origin);
    Point { x: 0, y: 0 }
}

/// Infers an integer type.
pub fn inferred() -> i32 {
    let value = 42;
    value
}

/// Deliberately unformatted.
pub fn messy( )->i32{ 1+1 }

macro_rules! double {
    ($e:expr) => { $e * 2 };
}

/// Uses a local macro.
pub fn use_double() -> i32 {
    double!(21)
}

/// Builds a vector.
pub fn make_vec() -> Vec<i32> {
    vec![1, 2, 3]
}
