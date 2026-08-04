use std::fmt;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct StartupInput {
    _private: (),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupError {
    NoSupportedConfiguration,
}

impl fmt::Display for StartupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSupportedConfiguration => formatter.write_str("no supported configuration"),
        }
    }
}

pub fn validate_startup(_input: StartupInput) -> Result<(), StartupError> {
    Err(StartupError::NoSupportedConfiguration)
}

#[cfg(test)]
mod tests {
    use super::{validate_startup, StartupError, StartupInput};

    #[test]
    fn default_startup_input_is_rejected() {
        assert_eq!(
            validate_startup(StartupInput::default()),
            Err(StartupError::NoSupportedConfiguration)
        );
    }

    #[test]
    fn unsupported_configuration_error_has_exact_display_text() {
        assert_eq!(
            StartupError::NoSupportedConfiguration.to_string(),
            "no supported configuration"
        );
    }
}
