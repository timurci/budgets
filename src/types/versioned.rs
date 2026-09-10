#[derive(Clone, Debug, PartialEq)]
pub struct Versioned<T> {
    pub value: T,
    pub version: u64,
}

impl<T> Versioned<T> {
    pub fn new(value: T) -> Self {
        Self { value, version: 0 }
    }

    pub fn at(version: u64, value: T) -> Self {
        Self { value, version }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_at_version_zero() {
        assert_eq!(
            Versioned::new("value"),
            Versioned {
                value: "value",
                version: 0
            }
        );
    }

    #[test]
    fn at_sets_version() {
        assert_eq!(
            Versioned::at(3, 7),
            Versioned {
                value: 7,
                version: 3
            }
        );
    }
}
