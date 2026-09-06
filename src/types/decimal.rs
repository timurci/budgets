use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Div, Mul, Sub, SubAssign};
use std::str::FromStr;

use thiserror::Error;

const SCALE: u8 = 8;
const SCALE_FACTOR: u64 = 100_000_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Decimal(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ParseDecimalError {
    #[error("invalid decimal format")]
    InvalidFormat,
    #[error("too many decimal places (maximum {})", SCALE)]
    TooManyDecimalPlaces,
    #[error("negative values are not supported")]
    NegativeValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum DecimalError {
    #[error("decimal addition overflow")]
    Overflow,
    #[error("decimal subtraction underflow")]
    Underflow,
}

impl Decimal {
    pub const ZERO: Self = Self(0);

    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn as_raw(&self) -> u64 {
        self.0
    }

    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn safe_add(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Overflow)
    }

    pub fn safe_sub(self, rhs: Self) -> Result<Self, DecimalError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(DecimalError::Underflow)
    }
}

macro_rules! impl_from_unsigned {
    ($($t:ty),* $(,)?) => {
        $(
            impl From<$t> for Decimal {
                fn from(value: $t) -> Self {
                    Self(u64::from(value) * SCALE_FACTOR)
                }
            }
        )*
    };
}

impl_from_unsigned!(u8, u16, u32, u64);

impl Add for Decimal {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl Sub for Decimal {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self(self.0 - rhs.0)
    }
}

impl AddAssign for Decimal {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl SubAssign for Decimal {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0;
    }
}

macro_rules! impl_scalar_mul {
    ($($t:ty),* $(,)?) => {
        $(
            impl Mul<$t> for Decimal {
                type Output = Self;

                fn mul(self, rhs: $t) -> Self {
                    Self(self.0 * u64::from(rhs))
                }
            }
        )*
    };
}

macro_rules! impl_scalar_div {
    ($($t:ty),* $(,)?) => {
        $(
            impl Div<$t> for Decimal {
                type Output = Self;

                fn div(self, rhs: $t) -> Self {
                    Self(self.0 / u64::from(rhs))
                }
            }
        )*
    };
}

impl_scalar_mul!(u8, u16, u32, u64);
impl_scalar_div!(u8, u16, u32, u64);

impl Sum for Decimal {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |total, value| total + value)
    }
}

impl<'a> Sum<&'a Decimal> for Decimal {
    fn sum<I: Iterator<Item = &'a Decimal>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |total, value| total + *value)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let units = self.0 / SCALE_FACTOR;
        let fraction = self.0 % SCALE_FACTOR;
        let width = usize::from(SCALE);
        write!(f, "{units}.{fraction:0width$}")
    }
}

impl FromStr for Decimal {
    type Err = ParseDecimalError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with('-') {
            return Err(ParseDecimalError::NegativeValue);
        }
        if !s.starts_with(|c: char| c.is_ascii_digit()) {
            return Err(ParseDecimalError::InvalidFormat);
        }
        let (units_part, fraction_part) = match s.split_once('.') {
            Some((units, fraction)) => (units, Some(fraction)),
            None => (s, None),
        };
        let units = units_part
            .parse::<u64>()
            .map_err(|_| ParseDecimalError::InvalidFormat)?;
        let fraction = match fraction_part {
            Some(fraction) => parse_fraction(fraction)?,
            None => 0,
        };
        let raw = units
            .checked_mul(SCALE_FACTOR)
            .and_then(|raw| raw.checked_add(fraction))
            .ok_or(ParseDecimalError::InvalidFormat)?;
        Ok(Self(raw))
    }
}

fn parse_fraction(fraction: &str) -> Result<u64, ParseDecimalError> {
    if fraction.is_empty() {
        return Err(ParseDecimalError::InvalidFormat);
    }
    if fraction.len() > usize::from(SCALE) {
        return Err(ParseDecimalError::TooManyDecimalPlaces);
    }
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ParseDecimalError::InvalidFormat);
    }
    let mut value = fraction
        .parse::<u64>()
        .map_err(|_| ParseDecimalError::InvalidFormat)?;
    for _ in fraction.len()..usize::from(SCALE) {
        value *= 10;
    }
    Ok(value)
}

#[cfg(test)]
mod construction_tests {
    use super::*;

    #[test]
    fn from_u64_scales_value() {
        assert_eq!(Decimal::from(5_u64), Decimal::new(500_000_000));
    }

    #[test]
    fn from_u16_scales_value() {
        assert_eq!(Decimal::from(5_u16), Decimal::new(500_000_000));
    }

    #[test]
    fn from_u32_scales_value() {
        assert_eq!(Decimal::from(5_u32), Decimal::new(500_000_000));
    }

    #[test]
    fn from_u8_scales_value() {
        assert_eq!(Decimal::from(5_u8), Decimal::new(500_000_000));
    }

    #[test]
    fn from_raw_matches_new() {
        assert_eq!(Decimal::from_raw(500_000_000), Decimal::new(500_000_000));
    }

    #[test]
    fn default_is_zero() {
        assert_eq!(Decimal::default(), Decimal::ZERO);
    }

    #[test]
    fn as_raw_exposes_internal_value() {
        assert_eq!(Decimal::new(123).as_raw(), 123);
    }

    #[test]
    fn is_zero_true_for_zero() {
        assert!(Decimal::ZERO.is_zero());
        assert!(Decimal::default().is_zero());
    }

    #[test]
    fn is_zero_false_for_nonzero() {
        assert!(!Decimal::from(1_u64).is_zero());
    }
}

#[cfg(test)]
mod arithmetic_tests {
    use super::*;

    #[test]
    fn add_sums_raw_values() {
        assert_eq!(
            Decimal::from(5_u64) + Decimal::from(3_u64),
            Decimal::from(8_u64)
        );
    }

    #[test]
    fn add_keeps_fractional_precision() {
        assert_eq!(
            Decimal::new(123_456_789_012) + Decimal::new(1),
            Decimal::new(123_456_789_013)
        );
    }

    #[test]
    fn sub_subtracts_raw_values() {
        assert_eq!(
            Decimal::from(5_u64) - Decimal::from(3_u64),
            Decimal::from(2_u64)
        );
    }

    #[test]
    fn sub_to_zero_is_zero() {
        let result = Decimal::from(5_u64) - Decimal::from(5_u64);
        assert!(result.is_zero());
    }

    #[test]
    fn safe_add_returns_sum() {
        assert_eq!(
            Decimal::from(5_u64).safe_add(Decimal::from(3_u64)),
            Ok(Decimal::from(8_u64))
        );
    }

    #[test]
    fn safe_add_overflow_fails() {
        assert_eq!(
            Decimal::new(u64::MAX).safe_add(Decimal::new(1)),
            Err(DecimalError::Overflow)
        );
    }

    #[test]
    fn safe_sub_returns_difference() {
        assert_eq!(
            Decimal::from(5_u64).safe_sub(Decimal::from(3_u64)),
            Ok(Decimal::from(2_u64))
        );
    }

    #[test]
    fn safe_sub_underflow_fails() {
        assert_eq!(
            Decimal::from(3_u64).safe_sub(Decimal::from(5_u64)),
            Err(DecimalError::Underflow)
        );
    }

    #[test]
    fn add_assign_accumulates() {
        let mut balance = Decimal::from(100_u64);
        balance += Decimal::from(50_u64);
        assert_eq!(balance, Decimal::from(150_u64));
    }

    #[test]
    fn sub_assign_deducts() {
        let mut balance = Decimal::from(100_u64);
        balance -= Decimal::from(30_u64);
        assert_eq!(balance, Decimal::from(70_u64));
    }

    #[test]
    fn mul_u8_multiplies_raw_value() {
        assert_eq!(Decimal::from(10_u64) * 3_u8, Decimal::from(30_u64));
    }

    #[test]
    fn mul_u8_keeps_scale() {
        assert_eq!(
            Decimal::new(250_000_000) * 4_u8,
            Decimal::new(1_000_000_000)
        );
    }

    #[test]
    fn mul_u16_keeps_scale() {
        assert_eq!(
            Decimal::new(250_000_000) * 4_u16,
            Decimal::new(1_000_000_000)
        );
    }

    #[test]
    fn div_u8_divides_raw_value() {
        assert_eq!(Decimal::from(30_u64) / 3_u8, Decimal::from(10_u64));
    }

    #[test]
    fn div_u16_truncates_at_smallest_unit() {
        assert_eq!(Decimal::new(SCALE_FACTOR) / 3_u16, Decimal::new(33_333_333));
    }

    #[test]
    fn div_u8_truncates_at_smallest_unit() {
        assert_eq!(Decimal::new(SCALE_FACTOR) / 3_u8, Decimal::new(33_333_333));
    }

    #[test]
    fn sum_of_decimals() {
        let values = vec![
            Decimal::from(1_u64),
            Decimal::from(2_u64),
            Decimal::from(3_u64),
        ];
        let total: Decimal = values.into_iter().sum();
        assert_eq!(total, Decimal::from(6_u64));
    }

    #[test]
    fn sum_of_empty_iterator_is_zero() {
        let total: Decimal = Vec::<Decimal>::new().into_iter().sum();
        assert!(total.is_zero());
    }

    #[test]
    fn sum_of_references() {
        let values = [Decimal::from(1_u64), Decimal::from(2_u64)];
        let total: Decimal = values.iter().sum();
        assert_eq!(total, Decimal::from(3_u64));
    }
}

#[cfg(test)]
mod comparison_tests {
    use super::*;

    #[test]
    fn equal_values_are_equal() {
        assert_eq!(Decimal::from(5_u64), Decimal::new(500_000_000));
    }

    #[test]
    fn different_values_are_not_equal() {
        assert_ne!(Decimal::from(5_u64), Decimal::from(6_u64));
    }

    #[test]
    fn ordering_by_raw_value() {
        assert!(Decimal::from(3_u64) < Decimal::from(5_u64));
        assert!(Decimal::from(5_u64) > Decimal::from(3_u64));
        assert!(Decimal::from(5_u64) <= Decimal::from(5_u64));
        assert!(Decimal::from(5_u64) >= Decimal::from(5_u64));
    }

    #[test]
    fn fractional_comparison() {
        assert!(Decimal::new(123_456_789_012) < Decimal::new(123_456_789_013));
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn zero_displays_with_full_precision() {
        assert_eq!(Decimal::ZERO.to_string(), "0.00000000");
    }

    #[test]
    fn whole_value_displays_with_full_precision() {
        assert_eq!(Decimal::from(1_u64).to_string(), "1.00000000");
    }

    #[test]
    fn fractional_value_displays_padded() {
        assert_eq!(Decimal::new(123_450_000_000).to_string(), "1234.50000000");
    }

    #[test]
    fn eight_digit_fraction_displays_exactly() {
        assert_eq!(Decimal::new(123_456_789_012).to_string(), "1234.56789012");
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn parses_whole_number() {
        assert_eq!("1234".parse::<Decimal>(), Ok(Decimal::from(1234_u64)));
    }

    #[test]
    fn parses_zero() {
        assert_eq!("0".parse::<Decimal>(), Ok(Decimal::ZERO));
    }

    #[test]
    fn parses_partial_fraction_padded() {
        assert_eq!(
            "1234.5".parse::<Decimal>(),
            Ok(Decimal::new(123_450_000_000))
        );
    }

    #[test]
    fn parses_full_fraction() {
        assert_eq!(
            "1234.56789012".parse::<Decimal>(),
            Ok(Decimal::new(123_456_789_012))
        );
    }

    #[test]
    fn parses_trailing_zeros_in_fraction() {
        assert_eq!(
            "1234.5000".parse::<Decimal>(),
            Ok(Decimal::new(123_450_000_000))
        );
    }

    #[test]
    fn parses_leading_zero_fraction() {
        assert_eq!("0.00000001".parse::<Decimal>(), Ok(Decimal::new(1)));
    }

    #[test]
    fn parses_max_boundary_value() {
        assert_eq!(
            "184467440737.09551615".parse::<Decimal>(),
            Ok(Decimal::new(u64::MAX))
        );
    }

    #[test]
    fn rejects_too_many_decimal_places() {
        assert_eq!(
            "1234.567890123".parse::<Decimal>(),
            Err(ParseDecimalError::TooManyDecimalPlaces)
        );
    }

    #[test]
    fn rejects_negative_values() {
        assert_eq!(
            "-5".parse::<Decimal>(),
            Err(ParseDecimalError::NegativeValue)
        );
    }

    #[test]
    fn rejects_empty_string() {
        assert_eq!("".parse::<Decimal>(), Err(ParseDecimalError::InvalidFormat));
    }

    #[test]
    fn rejects_non_numeric_input() {
        assert_eq!(
            "abc".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_leading_whitespace() {
        assert_eq!(
            " 1234".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_empty_fraction() {
        assert_eq!(
            "1234.".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_missing_units() {
        assert_eq!(
            ".5".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_second_decimal_point() {
        assert_eq!(
            "1.2.3".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_plus_prefix() {
        assert_eq!(
            "+5".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_comma_separator() {
        assert_eq!(
            "1,5".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn rejects_whole_units_overflow() {
        assert_eq!(
            "184467440738.0".parse::<Decimal>(),
            Err(ParseDecimalError::InvalidFormat)
        );
    }

    #[test]
    fn display_parse_roundtrip() {
        let value = Decimal::new(123_456_789_012);
        assert_eq!(value.to_string().parse::<Decimal>(), Ok(value));
    }
}
