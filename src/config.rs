use eyre::bail;
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs::{read_to_string, write};
use std::io::ErrorKind;
use std::path::PathBuf;
use tracing::error;

pub struct Config<V: Serialize + DeserializeOwned + Default> {
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
    
    pub fn flush_changes(&mut self) {
        if let Some(value) = &self.value {
            let string = serde_json::to_string(value).unwrap();
            write(&self.path, string).unwrap();
        }
    }

    fn load_from_file(&mut self) -> eyre::Result<V> {
        let string = match read_to_string(&self.path) {
            Ok(value) => value,
            Err(error) => {
                if error.kind() == ErrorKind::NotFound {
                    return Ok(V::default());
                }

                bail!(error);
            }
        };

        let value: V = match serde_json::from_str(&string) {
            Ok(value) => value,
            Err(error) => {
                error!("Could not load config file: {error:?}");
                V::default()
            }
        };

        Ok(value)
    }
}
