macro_rules! id_type {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
            pub struct $name(uuid::Uuid);

            impl $name {
                pub const fn new(value: uuid::Uuid) -> Self {
                    Self(value)
                }

                pub fn new_v7() -> Self {
                    Self(uuid::Uuid::now_v7())
                }

                pub const fn as_uuid(&self) -> uuid::Uuid {
                    self.0
                }
            }

            impl std::fmt::Display for $name {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "{}", self.0)
                }
            }

            impl From<uuid::Uuid> for $name {
                fn from(value: uuid::Uuid) -> Self {
                    Self(value)
                }
            }
        )+
    };
}

pub(crate) use id_type;
