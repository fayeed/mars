const ASSIGNMENT_KEYS: [&str; 9] = [
    "API_KEY",
    "ACCESS_TOKEN",
    "AUTH_TOKEN",
    "PASSWORD",
    "SECRET_KEY",
    "CLIENT_SECRET",
    "PRIVATE_TOKEN",
    "GITHUB_TOKEN",
    "OPENAI_API_KEY",
];

#[must_use]
pub fn detect_secret(input: &str) -> Option<&'static str> {
    let upper = input.to_ascii_uppercase();
    if upper.contains("-----BEGIN") && upper.contains("PRIVATE KEY-----") {
        return Some("private key material");
    }
    if upper.lines().any(has_sensitive_assignment) {
        return Some("credential assignment");
    }
    if input.split(is_token_boundary).any(is_secret_token) {
        return Some("API token");
    }
    if upper.lines().any(|line| {
        line.trim_start()
            .strip_prefix("AUTHORIZATION:")
            .is_some_and(|value| value.trim_start().starts_with("BEARER "))
    }) {
        return Some("authorization bearer token");
    }
    None
}

#[must_use]
pub fn redact(input: &str) -> String {
    let without_keys = redact_private_keys(input);
    without_keys
        .lines()
        .map(redact_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn redact_private_keys(input: &str) -> String {
    let mut output = Vec::new();
    let mut private_key = false;
    for line in input.lines() {
        let upper = line.to_ascii_uppercase();
        if upper.contains("-----BEGIN") && upper.contains("PRIVATE KEY-----") {
            if !private_key {
                output.push("[REDACTED PRIVATE KEY]");
            }
            private_key = true;
            continue;
        }
        if private_key {
            if upper.contains("-----END") && upper.contains("PRIVATE KEY-----") {
                private_key = false;
            }
            continue;
        }
        output.push(line);
    }
    output.join("\n")
}

fn redact_line(line: &str) -> String {
    let upper = line.to_ascii_uppercase();
    if let Some(index) = upper.find("AUTHORIZATION:")
        && upper[index + "AUTHORIZATION:".len()..]
            .trim_start()
            .starts_with("BEARER ")
    {
        return format!("{}Authorization: Bearer [REDACTED]", &line[..index]);
    }
    if has_sensitive_assignment(&upper)
        && let Some(index) = line.find(['=', ':'])
    {
        return format!("{}{}[REDACTED]", &line[..index], &line[index..=index]);
    }

    let mut output = String::with_capacity(line.len());
    let mut start = 0;
    for (index, character) in line.char_indices() {
        if is_token_boundary(character) {
            redact_token(&mut output, &line[start..index]);
            output.push(character);
            start = index + character.len_utf8();
        }
    }
    redact_token(&mut output, &line[start..]);
    output
}

fn redact_token(output: &mut String, token: &str) {
    if is_secret_token(token) {
        output.push_str("[REDACTED]");
    } else {
        output.push_str(token);
    }
}

fn has_sensitive_assignment(line: &str) -> bool {
    ASSIGNMENT_KEYS
        .iter()
        .any(|key| line.contains(key) && (line.contains('=') || line.contains(':')))
}

fn is_token_boundary(character: char) -> bool {
    !character.is_ascii_alphanumeric() && !matches!(character, '_' | '-')
}

fn is_secret_token(token: &str) -> bool {
    (token.starts_with("ghp_") && token.len() >= 20)
        || (token.starts_with("github_pat_") && token.len() >= 20)
        || (token.starts_with("sk-") && token.len() >= 20)
        || (token.starts_with("AKIA")
            && token.len() == 20
            && token
                .chars()
                .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_tokens_assignments_bearer_headers_and_private_keys() {
        let input = "ghp_abcdefghijklmnopqrstuvwxyz\nOPENAI_API_KEY=sk-abcdefghijklmnopqrstuvwxyz\nAuthorization: Bearer abc.def.ghi\n-----BEGIN PRIVATE KEY-----\nsecret-material\n-----END PRIVATE KEY-----\nsafe";
        let redacted = redact(input);
        assert!(!redacted.contains("abcdefghijklmnopqrstuvwxyz"));
        assert!(!redacted.contains("abc.def.ghi"));
        assert!(!redacted.contains("secret-material"));
        assert!(redacted.contains("OPENAI_API_KEY=[REDACTED]"));
        assert!(redacted.contains("Authorization: Bearer [REDACTED]"));
        assert!(redacted.ends_with("safe"));
    }
}
