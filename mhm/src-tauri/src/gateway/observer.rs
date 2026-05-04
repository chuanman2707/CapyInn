use axum::http::HeaderMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverErrorCode {
    InvalidCursor,
    CursorExpired,
    ObserverUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverError {
    pub code: ObserverErrorCode,
    pub message: String,
}

pub fn parse_cursor(
    query_last_event_id: Option<&str>,
    headers: &HeaderMap,
) -> Result<Option<i64>, ObserverError> {
    let cursor = match query_last_event_id {
        Some(cursor) => Some(cursor),
        None => headers
            .get("Last-Event-ID")
            .map(|value| value.to_str())
            .transpose()
            .map_err(|_| invalid_cursor("Last-Event-ID header must be a valid integer cursor"))?,
    };

    cursor.map(parse_cursor_value).transpose()
}

fn parse_cursor_value(cursor: &str) -> Result<i64, ObserverError> {
    let parsed = cursor
        .parse::<i64>()
        .map_err(|_| invalid_cursor("Cursor must be a non-negative integer"))?;

    if parsed < 0 {
        return Err(invalid_cursor("Cursor must be a non-negative integer"));
    }

    Ok(parsed)
}

fn invalid_cursor(message: &str) -> ObserverError {
    ObserverError {
        code: ObserverErrorCode::InvalidCursor,
        message: message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cursor, ObserverErrorCode};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn parse_cursor_missing_cursor_returns_none() {
        let headers = HeaderMap::new();

        let cursor = parse_cursor(None, &headers).expect("missing cursor is valid");

        assert_eq!(cursor, None);
    }

    #[test]
    fn parse_cursor_reads_query_last_event_id() {
        let headers = HeaderMap::new();

        let cursor = parse_cursor(Some("42"), &headers).expect("query cursor is valid");

        assert_eq!(cursor, Some(42));
    }

    #[test]
    fn parse_cursor_reads_last_event_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("84"));

        let cursor = parse_cursor(None, &headers).expect("header cursor is valid");

        assert_eq!(cursor, Some(84));
    }

    #[test]
    fn parse_cursor_query_wins_over_last_event_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("84"));

        let cursor = parse_cursor(Some("42"), &headers).expect("query cursor is valid");

        assert_eq!(cursor, Some(42));
    }

    #[test]
    fn parse_cursor_rejects_negative_query_cursor() {
        let headers = HeaderMap::new();

        let error = parse_cursor(Some("-1"), &headers).expect_err("negative cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_non_numeric_query_cursor() {
        let headers = HeaderMap::new();

        let error = parse_cursor(Some("abc"), &headers).expect_err("non-numeric cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_negative_header_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("-1"));

        let error = parse_cursor(None, &headers).expect_err("negative cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_non_numeric_header_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("abc"));

        let error = parse_cursor(None, &headers).expect_err("non-numeric cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }
}
