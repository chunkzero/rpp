use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Config {
    pub rpp: RppSection,
    pub server: ServerSection,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RppSection {
    pub root_dir: String,
    pub plugins: Vec<PluginDef>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerSection {
    pub host: Ipv4Addr,
    pub port: u16,
    pub default_pack: String,
}

#[derive(Debug, Clone)]
pub enum PluginDef {
    Local {
        id: String,
    },
    Remote {
        repository: String,
        id: String,
        version: String,
    },
}

// Could impl FromString trait and Display trait, like how you did with serialize and deserialize
impl PluginDef {
    // TODO: replace anyhow with crate err
    pub fn from_string(value: &str) -> anyhow::Result<Self> {
        if let Some(id) = value.strip_prefix("local:") {
            Ok(PluginDef::Local { id: id.to_string() })
        } else {
            let parts: Vec<&str> = value.splitn(3, ':').collect();
            if parts.len() == 3 {
                Ok(PluginDef::Remote {
                    repository: parts[0].to_string(),
                    id: parts[1].to_string(),
                    version: parts[2].to_string(),
                })
            } else {
                Err(anyhow::anyhow!("invalid plugin definition: {}", value))
            }
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Local { id } => format!("local:{}", &id),
            Self::Remote {
                repository,
                id,
                version,
            } => format!("{}:{}:{}", &repository, &id, &version),
        }
    }
}

impl Serialize for PluginDef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.as_string())
    }
}
// I tend to use 'a for everything (unless there is more than one) but this isn't neceasrily bad.
impl<'de> Deserialize<'de> for PluginDef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct PluginDefVisitor;

        impl<'de> serde::de::Visitor<'de> for PluginDefVisitor {
            type Value = PluginDef;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str(
                    "a plugin definition string in format \
                     'local:{id}' or '{repository}:{id}:{version}'",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                PluginDef::from_string(value).map_err(E::custom)
            }
        }

        deserializer.deserialize_str(PluginDefVisitor)
    }
}
