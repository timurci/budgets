use std::collections::HashMap;

use crate::types::Decimal;

use super::model::{CategoryId, DebtId};

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AccountEvent {
    CategoryAdded {
        id: CategoryId,
        allocation: u8,
    },
    CategoryRemoved {
        id: CategoryId,
    },
    RolledOver {
        income: Decimal,
    },
    Spent {
        category: CategoryId,
        amount: Decimal,
    },
    SurplusTransferred {
        from: CategoryId,
        to: CategoryId,
        amount: Decimal,
    },
    FundsAdded {
        amount: Decimal,
    },
    FundsRemoved {
        amount: Decimal,
    },
    CategoriesReallocated {
        allocations: HashMap<CategoryId, u8>,
    },
    DebtAdded {
        id: DebtId,
        amount: Decimal,
    },
    DebtRepaid {
        category: CategoryId,
        debt: DebtId,
        amount: Decimal,
    },
}

#[cfg(test)]
mod serde_tests {
    use super::*;

    fn category(seed: u128) -> CategoryId {
        CategoryId::from(uuid::Uuid::from_u128(seed))
    }

    #[test]
    fn event_json_format_is_stable() {
        assert_eq!(
            serde_json::to_string(&AccountEvent::FundsAdded {
                amount: Decimal::from(10000u64),
            })
            .unwrap(),
            r#"{"FundsAdded":{"amount":1000000000000}}"#
        );
        assert_eq!(
            serde_json::to_string(&AccountEvent::Spent {
                category: category(1),
                amount: Decimal::from(400u64),
            })
            .unwrap(),
            r#"{"Spent":{"category":"00000000-0000-0000-0000-000000000001","amount":40000000000}}"#
        );
    }

    #[test]
    fn event_json_decodes() {
        let decoded: AccountEvent =
            serde_json::from_str(r#"{"FundsAdded":{"amount":1000000000000}}"#).unwrap();
        assert_eq!(
            decoded,
            AccountEvent::FundsAdded {
                amount: Decimal::from(10000u64),
            }
        );
    }

    #[test]
    fn decimal_json_format_is_raw_scaled_integer() {
        assert_eq!(
            serde_json::to_string(&Decimal::new(123_456_789_012)).unwrap(),
            "123456789012"
        );
    }
}
