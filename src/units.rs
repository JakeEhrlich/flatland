//! Lengths are stored internally as integer nanometres so geometry is exact.
//! In JSON a length is a bare number (millimetres) or a string with a unit:
//! `"0.25mm"`, `"10mil"`, `"0.1in"`, `"100um"`.

use crate::error::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::ops::{Add, Div, Mul, Neg, Sub};

pub const NM_PER_MM: i64 = 1_000_000;
pub const NM_PER_UM: i64 = 1_000;

/// A length in nanometres.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Length(pub i64);

impl Length {
    pub const ZERO: Length = Length(0);

    pub fn from_mm(mm: f64) -> Length {
        Length((mm * NM_PER_MM as f64).round() as i64)
    }
    pub fn from_nm(nm: i64) -> Length {
        Length(nm)
    }
    pub fn mm(self) -> f64 {
        self.0 as f64 / NM_PER_MM as f64
    }
    pub fn nm(self) -> i64 {
        self.0
    }
    pub fn um(self) -> f64 {
        self.0 as f64 / NM_PER_UM as f64
    }
    pub fn inches(self) -> f64 {
        self.mm() / 25.4
    }
    pub fn abs(self) -> Length {
        Length(self.0.abs())
    }
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Parse `"1.5"`, `"1.5mm"`, `"10mil"`, `"0.1in"`, `"100um"`, `"2cm"`.
    pub fn parse(s: &str) -> Result<Length> {
        let s = s.trim();
        let split = s
            .find(|c: char| c.is_ascii_alphabetic())
            .unwrap_or(s.len());
        let (num, unit) = s.split_at(split);
        let num = num.trim();
        let value: f64 = num.parse().map_err(|_| {
            Error::with_help(
                format!("`{s}` is not a length"),
                "lengths look like `1.5` (mm), `1.5mm`, `10mil`, `0.1in`, `100um`",
            )
        })?;
        let mm = match unit.trim().to_ascii_lowercase().as_str() {
            "" | "mm" => value,
            "cm" => value * 10.0,
            "m" => value * 1000.0,
            "um" | "µm" => value / 1000.0,
            "nm" => value / 1e6,
            "mil" | "th" | "thou" => value * 0.0254,
            "in" | "inch" | "\"" => value * 25.4,
            other => {
                return Err(Error::with_help(
                    format!("unknown length unit `{other}` in `{s}`"),
                    "supported units: mm (default), cm, m, um, nm, mil, in",
                ))
            }
        };
        Ok(Length::from_mm(mm))
    }
}

impl fmt::Debug for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}mm", fmt_mm(self.mm()))
    }
}
impl fmt::Display for Length {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}mm", fmt_mm(self.mm()))
    }
}

/// Format millimetres compactly without trailing zeros.
pub fn fmt_mm(mm: f64) -> String {
    let s = format!("{mm:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" { "0".into() } else { s.to_string() }
}

impl Add for Length {
    type Output = Length;
    fn add(self, o: Length) -> Length { Length(self.0 + o.0) }
}
impl Sub for Length {
    type Output = Length;
    fn sub(self, o: Length) -> Length { Length(self.0 - o.0) }
}
impl Neg for Length {
    type Output = Length;
    fn neg(self) -> Length { Length(-self.0) }
}
impl Mul<i64> for Length {
    type Output = Length;
    fn mul(self, o: i64) -> Length { Length(self.0 * o) }
}
impl Mul<f64> for Length {
    type Output = Length;
    fn mul(self, o: f64) -> Length { Length((self.0 as f64 * o).round() as i64) }
}
impl Div<i64> for Length {
    type Output = Length;
    fn div(self, o: i64) -> Length { Length(self.0 / o) }
}

impl Serialize for Length {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        // Serialize as mm; integers stay integers so files look clean.
        let mm = self.mm();
        if mm.fract() == 0.0 && mm.abs() < 1e15 {
            s.serialize_i64(mm as i64)
        } else {
            s.serialize_f64(mm)
        }
    }
}

impl<'de> Deserialize<'de> for Length {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Num(f64),
            Str(String),
        }
        match Raw::deserialize(d)? {
            Raw::Num(n) => Ok(Length::from_mm(n)),
            Raw::Str(s) => Length::parse(&s).map_err(serde::de::Error::custom),
        }
    }
}

/// A point in board coordinates (nanometres). Serialized as `[x, y]` in mm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Point {
    pub x: Length,
    pub y: Length,
}

impl Point {
    pub const ORIGIN: Point = Point { x: Length::ZERO, y: Length::ZERO };
    pub fn new(x: Length, y: Length) -> Point { Point { x, y } }
    pub fn mm(x: f64, y: f64) -> Point { Point { x: Length::from_mm(x), y: Length::from_mm(y) } }
    pub fn nm(x: i64, y: i64) -> Point { Point { x: Length(x), y: Length(y) } }
    pub fn xy_mm(self) -> (f64, f64) { (self.x.mm(), self.y.mm()) }
    pub fn distance(self, o: Point) -> Length {
        let dx = (self.x.0 - o.x.0) as f64;
        let dy = (self.y.0 - o.y.0) as f64;
        Length((dx * dx + dy * dy).sqrt().round() as i64)
    }
    /// Parse `"x,y"` (mm or with units per coordinate).
    pub fn parse(s: &str) -> Result<Point> {
        let parts: Vec<&str> = s.split(',').collect();
        if parts.len() != 2 {
            return Err(Error::with_help(
                format!("`{s}` is not a point"),
                "points look like `x,y`, e.g. `12.5,3` or `0.5in,10mm`",
            ));
        }
        Ok(Point { x: Length::parse(parts[0])?, y: Length::parse(parts[1])? })
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point { Point { x: self.x + o.x, y: self.y + o.y } }
}
impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point { Point { x: self.x - o.x, y: self.y - o.y } }
}

impl fmt::Debug for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "({}, {})", fmt_mm(self.x.mm()), fmt_mm(self.y.mm()))
    }
}
impl fmt::Display for Point {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{},{}", fmt_mm(self.x.mm()), fmt_mm(self.y.mm()))
    }
}

impl Serialize for Point {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        [self.x, self.y].serialize(s)
    }
}
impl<'de> Deserialize<'de> for Point {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let [x, y] = <[Length; 2]>::deserialize(d)?;
        Ok(Point { x, y })
    }
}

/// Parse a rotation in degrees (`"90"`, `"45deg"`, `"-90"`).
pub fn parse_degrees(s: &str) -> Result<f64> {
    let t = s.trim().trim_end_matches("deg").trim();
    t.parse::<f64>()
        .map_err(|_| Error::with_help(format!("`{s}` is not an angle"), "angles are in degrees, e.g. `90` or `-45`"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_lengths() {
        assert_eq!(Length::parse("1.5").unwrap(), Length::from_mm(1.5));
        assert_eq!(Length::parse("1.5mm").unwrap(), Length::from_mm(1.5));
        assert_eq!(Length::parse("10mil").unwrap(), Length::from_mm(0.254));
        assert_eq!(Length::parse("1in").unwrap(), Length::from_mm(25.4));
        assert_eq!(Length::parse("100um").unwrap(), Length(100_000));
        assert!(Length::parse("3 furlongs").is_err());
    }
    #[test]
    fn json_roundtrip() {
        let p: Point = serde_json::from_str("[1.5, \"10mil\"]").unwrap();
        assert_eq!(p, Point::new(Length::from_mm(1.5), Length::from_mm(0.254)));
        assert_eq!(serde_json::to_string(&Length::from_mm(2.0)).unwrap(), "2");
        assert_eq!(serde_json::to_string(&Length::from_mm(0.25)).unwrap(), "0.25");
    }
}
