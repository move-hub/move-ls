use move_core_types::account_address::AccountAddress;
use move_lang::shared::Address;
use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};
use std::{convert::TryFrom, path::PathBuf};

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub dialect: String,
    pub stdlib_folder: Option<PathBuf>,
    pub modules_folders: Vec<PathBuf>,
    #[serde(deserialize_with = "deserialize_address")]
    #[serde(serialize_with = "serialize_address")]
    pub sender_address: Option<Address>,
}

fn deserialize_address<'de, D>(d: D) -> Result<Option<Address>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = <Option<String>>::deserialize(d)?;

    s.filter(|e| !e.trim().is_empty())
        .map(|e| {
            AccountAddress::from_hex_literal(e.as_str())
                .map(|a| Address::try_from(a.as_ref()).unwrap())
        })
        .transpose()
        .map_err(|err| D::Error::custom(err))
}
fn serialize_address<S>(
    addr: &Option<Address>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    addr.map(|a| a.to_string()).serialize(serializer)
}

#[cfg(test)]
mod tests {
    use crate::config::ProjectConfig;

    #[test]
    fn test_config_parse() {
        let source = r#"
    {
        "modules_folders": [
            "/Users/annali007/projects/lerencao/token-swap/src/modules"
        ],
        "dialect": "starcoin",
        "stdlib_folder": "/Users/annali007/projects/starcoin/vm/stdlib/modules",
        "sender_address": "0x42"
    }
        "#;

        let config: ProjectConfig = serde_json::from_str(source).unwrap();

        let source = r#"
    {
        "modules_folders": [
            "/Users/annali007/projects/lerencao/token-swap/src/modules"
        ],
        "dialect": "starcoin",
        "stdlib_folder": "/Users/annali007/projects/starcoin/vm/stdlib/modules",
        "sender_address": ""
    }
        "#;

        let config: ProjectConfig = serde_json::from_str(source).unwrap();
        assert!(config.sender_address.is_none());

        let source = r#"
    {
        "modules_folders": [
            "/Users/annali007/projects/lerencao/token-swap/src/modules"
        ],
        "dialect": "starcoin",
        "stdlib_folder": "",
        "sender_address": ""
    }
        "#;

        let config: ProjectConfig = serde_json::from_str(source).unwrap();
        assert_eq!(config.stdlib_folder, None);
    }
}
