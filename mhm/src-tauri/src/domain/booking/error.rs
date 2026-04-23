use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BookingError {
    Conflict(String),
    NotFound(String),
    Validation(String),
    Database(String),
    DatabaseWrite(String),
    DateTimeParse(String),
}

pub type BookingResult<T> = Result<T, BookingError>;

impl BookingError {
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation(message.into())
    }

    pub fn database(message: impl Into<String>) -> Self {
        Self::Database(message.into())
    }

    pub fn database_write(message: impl Into<String>) -> Self {
        Self::DatabaseWrite(message.into())
    }

    pub fn datetime_parse(message: impl Into<String>) -> Self {
        Self::DateTimeParse(message.into())
    }
}

impl fmt::Display for BookingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict(message)
            | Self::NotFound(message)
            | Self::Validation(message)
            | Self::Database(message)
            | Self::DatabaseWrite(message)
            | Self::DateTimeParse(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for BookingError {}

impl From<sqlx::Error> for BookingError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::BookingError;

    #[test]
    fn booking_error_formats_conflict_messages() {
        let error = BookingError::Conflict("room already occupied".to_string());

        assert_eq!(error.to_string(), "room already occupied");
    }

    #[test]
    fn booking_error_formats_database_write_messages() {
        let error = BookingError::database_write("disk full");

        assert_eq!(error.to_string(), "disk full");
    }
}
