//! A local, deterministic embedding for the semantic layer.
//!
//! Phase 0 forbids cloud and model calls, so this is a feature-hashing embedder
//! over code-aware tokens rather than a learned model. It captures identifier
//! and path overlap — enough for semantic recall to complement lexical search —
//! and it is reproducible, so a chunk's vector never changes unless its text
//! does. The model name is stored with every vector, so a learned embedder can
//! be introduced later without invalidating the schema.

/// Identifies the vectors this embedder produces.
pub const MODEL: &str = "mars-hashing-v1";

/// Vector width. Small enough to scan a large repository in memory.
pub const DIMENSIONS: usize = 256;

/// Embeds text into a unit-length vector.
#[must_use]
pub fn embed(text: &str) -> Vec<f32> {
    let tokens = tokenize(text);
    let mut vector = vec![0.0_f32; DIMENSIONS];

    for token in &tokens {
        add_feature(&mut vector, token, 1.0);
        // Sub-token trigrams let related identifiers land near each other.
        for trigram in trigrams(token) {
            add_feature(&mut vector, &trigram, 0.4);
        }
    }
    for pair in tokens.windows(2) {
        add_feature(&mut vector, &format!("{}_{}", pair[0], pair[1]), 0.6);
    }

    normalize(&mut vector);
    vector
}

/// Splits text into lowercase tokens, breaking identifiers on case and
/// punctuation so `parseConfig`, `parse_config`, and `ParseConfig` agree.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    let flush = |current: &mut String, tokens: &mut Vec<String>| {
        if current.len() >= 2 {
            tokens.push(std::mem::take(current));
        } else {
            current.clear();
        }
    };

    let mut previous: Option<char> = None;
    for character in text.chars() {
        if character.is_alphanumeric() {
            let boundary = previous.is_some_and(|previous| {
                (previous.is_lowercase() || previous.is_numeric()) && character.is_uppercase()
            });
            if boundary {
                flush(&mut current, &mut tokens);
            }
            current.extend(character.to_lowercase());
        } else {
            flush(&mut current, &mut tokens);
        }
        previous = Some(character);
    }
    flush(&mut current, &mut tokens);
    tokens
}

fn trigrams(token: &str) -> Vec<String> {
    if token.len() < 4 {
        return Vec::new();
    }
    let characters: Vec<char> = token.chars().collect();
    characters
        .windows(3)
        .map(|window| window.iter().collect())
        .collect()
}

/// Feature hashing with a sign hash, so collisions cancel rather than compound.
fn add_feature(vector: &mut [f32], feature: &str, weight: f32) {
    let hash = fnv1a(feature.as_bytes());
    let bucket = (hash % DIMENSIONS as u64) as usize;
    let sign = if (hash >> 40) & 1 == 0 { 1.0 } else { -1.0 };
    vector[bucket] += sign * weight;
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn normalize(vector: &mut [f32]) {
    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > f32::EPSILON {
        for value in vector.iter_mut() {
            *value /= magnitude;
        }
    }
}

/// Encodes a vector for storage as little-endian `f32`.
#[must_use]
pub fn encode(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

/// Decodes a stored vector, returning `None` if the blob is not a whole number
/// of `f32` values.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

/// Cosine similarity of two unit vectors, clamped to `[-1, 1]`.
#[must_use]
pub fn similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() {
        return 0.0;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}
