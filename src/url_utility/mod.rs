use url::Url;

pub fn relative_url(url: &str) -> bool {
    match Url::parse(url) {
        Err(url::ParseError::RelativeUrlWithoutBase) => true,
        Ok(_) | Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::relative_url;

    #[test]
    fn relative_url_returns_false_for_full_url() {
        // arrange
        let url = "https://example.com/home.html";

        // act
        let result = relative_url(url);

        // assert
        assert!(!result);
    }

    #[test]
    fn relative_url_returns_true_for_relative_url() {
        // arrange
        let url = "/home.html";

        // act
        let result = relative_url(url);

        // assert
        assert!(result);
    }

    #[test]
    fn relative_url_returns_false_for_invalid_url() {
        // arrange
        let url = "http://[:::1]"; // DevSkim: ignore DS137138 - use of HTTP-based URL without TLS is in a unti test

        // act
        let result = relative_url(url);

        // assert
        assert!(!result);
    }
}
