use keyring::Entry;

use crate::ForgeError;

pub trait CredentialVault: Send + Sync {
    fn set(&self, credential_ref: &str, secret: &str) -> Result<(), ForgeError>;
    fn get(&self, credential_ref: &str) -> Result<String, ForgeError>;
    fn delete(&self, credential_ref: &str) -> Result<(), ForgeError>;
}

#[derive(Default)]
pub struct OsCredentialVault;

impl OsCredentialVault {
    fn entry(credential_ref: &str) -> Result<Entry, ForgeError> {
        Entry::new("mars-agent-ide", credential_ref)
            .map_err(|error| ForgeError::Credential(error.to_string()))
    }
}

impl CredentialVault for OsCredentialVault {
    fn set(&self, credential_ref: &str, secret: &str) -> Result<(), ForgeError> {
        Self::entry(credential_ref)?
            .set_password(secret)
            .map_err(|error| ForgeError::Credential(error.to_string()))
    }

    fn get(&self, credential_ref: &str) -> Result<String, ForgeError> {
        Self::entry(credential_ref)?
            .get_password()
            .map_err(|error| ForgeError::Credential(error.to_string()))
    }

    fn delete(&self, credential_ref: &str) -> Result<(), ForgeError> {
        Self::entry(credential_ref)?
            .delete_credential()
            .map_err(|error| ForgeError::Credential(error.to_string()))
    }
}
