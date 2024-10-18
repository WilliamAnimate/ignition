use eyre::bail;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{read_to_string, write};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use tracing::error;

pub struct Config<V> {
    value: Option<V>,
    path: PathBuf,
}

impl<V: Serialize + DeserializeOwned + Default> Config<V> {
    pub fn new(path: PathBuf) -> Self {
        Self { value: None, path }
    }

    pub fn get_mut(&mut self) -> eyre::Result<&mut V> {
        if self.value.is_none() {
            let v = self.load_from_file()?;
            self.value = Some(v);
        }

        Ok(self.value.as_mut().unwrap())
    }

    pub fn flush_changes(&mut self) -> eyre::Result<()>  {
        if let Some(value) = &self.value {
            Self::write_file(&self.path, value)?;
        }
        
        Ok(())
    }

    fn load_from_file(&mut self) -> eyre::Result<V> {
        Self::read_file(&self.path)
    }
  
}

impl<V: Serialize> Config<V> {
    pub fn write_file(path: &Path, value: &V) -> eyre::Result<()> {
        let string = serde_json::to_string(&value)?;
        write(path, string)?;
        Ok(())
    }
}

impl<V: DeserializeOwned + Default> Config<V> {
    pub fn read_file(path: &Path) -> eyre::Result<V> {
        let string = match read_to_string(&path) {
            Ok(value) => value,
            Err(error) => {
                if error.kind() == ErrorKind::NotFound {
                    return Ok(V::default());
                }

                bail!(error);
            }
        };

        let value: V = serde_json::from_str(&string).unwrap_or_else(|error| {
            error!("Could not load config file: {error:?}");
            V::default()
        });

        Ok(value)
    }
}