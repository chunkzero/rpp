use std::path::PathBuf;

use mlua::{FromLua, ObjectLike};
use regex::Regex;

#[derive(Debug, Clone)]
pub enum Filter {
    Path(Regex),
    FileName(Regex),
    All(Vec<Filter>),
    Any(Vec<Filter>),
    Negate(Box<Filter>),
}

impl FromLua for Filter {
    fn from_lua(value: mlua::Value, lua: &mlua::Lua) -> mlua::Result<Self> {
        let value = match value.as_table() {
            Some(table) => table,
            None => {
                return Err(mlua::Error::FromLuaConversionError {
                    from: value.type_name(),
                    to: "Filter".to_string(),
                    message: None,
                });
            }
        };

        let filter_type: String = value.get("type")?;

        match filter_type.as_str() {
            "path" => Ok(Filter::Path(
                Regex::new(&(value.get::<String>("regex")?))
                    .map_err(|err| mlua::Error::external(err))?,
            )),
            "filename" => Ok(Filter::FileName(
                Regex::new(&(value.get::<String>("regex")?))
                    .map_err(|err| mlua::Error::external(err))?,
            )),
            "all" => {
                let filters = value.get::<Vec<mlua::Table>>("filters")?;

                let mut filter_vec = Vec::with_capacity(2);

                for filter_table in filters {
                    filter_vec.push(Self::from_lua(filter_table.to_value(), lua)?);
                }

                Ok(Filter::All(filter_vec))
            }
            "any" => {
                let filters = value.get::<Vec<mlua::Table>>("filters")?;

                let mut filter_vec = Vec::with_capacity(2);

                for filter_table in filters {
                    filter_vec.push(Self::from_lua(filter_table.to_value(), lua)?);
                }

                Ok(Filter::Any(filter_vec))
            }
            "negate" => Ok(Filter::Negate(Box::from(Self::from_lua(
                value.get::<mlua::Table>("filter")?.to_value(),
                lua,
            )?))),
            filter => Err(mlua::Error::FromLuaConversionError {
                from: "table",
                to: "Filter".to_string(),
                message: Some(format!("Invalid filter type: {}", filter)),
            }),
        }
    }
}

impl Filter {
    pub fn filter(&self, relative_path: &PathBuf) -> crate::Result<bool> {
        match self {
            Filter::Path(regex) => Ok(regex.is_match(relative_path.to_string_lossy().as_ref())),
            Filter::FileName(regex) => Ok(regex.is_match(
                relative_path
                    .file_name()
                    .ok_or_else(|| crate::Error::Process("Unknown file name".into()))?
                    .to_string_lossy()
                    .as_ref(),
            )),
            Filter::All(filters) => {
                for filter in filters {
                    match filter.filter(relative_path) {
                        Ok(false) => return Ok(false),
                        Err(err) => return Err(err),
                        _ => {}
                    }
                }

                Ok(true)
            }
            Filter::Any(filters) => {
                for filter in filters {
                    match filter.filter(relative_path) {
                        Ok(true) => return Ok(true),
                        Err(err) => return Err(err),
                        _ => {}
                    }
                }

                Ok(false)
            }
            Filter::Negate(filter) => filter.filter(relative_path).map(|value| !value),
        }
    }
}
