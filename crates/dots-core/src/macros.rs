/// Declares a fieldless enum stored as a fixed string in SQLite and JSON.
macro_rules! str_enum {
    ($name:ident { $($variant:ident => $s:literal),+ $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
        pub enum $name {
            $( #[serde(rename = $s)] $variant ),+
        }

        impl $name {
            pub fn as_str(&self) -> &'static str {
                match self { $( Self::$variant => $s ),+ }
            }

            pub fn parse(s: &str) -> crate::Result<Self> {
                match s {
                    $( $s => Ok(Self::$variant), )+
                    other => Err(crate::Error::Invalid(format!(
                        "{}: unknown value {:?}", stringify!($name), other
                    ))),
                }
            }
        }
    };
}
