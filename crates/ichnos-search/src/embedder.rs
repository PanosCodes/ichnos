//! Turns text into vectors with a [Model2Vec](https://github.com/MinishLab/model2vec) static
//! embedding model.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use model2vec_rs::model::StaticModel;

use crate::SearchError;

/// The Hugging Face model that makes the vectors; downloaded into the Hugging Face cache on first
/// use. Changing it rebuilds the index.
pub(crate) const MODEL: &str = "minishlab/potion-base-8M";

/// Length of every vector [`MODEL`] makes; `index.sql` declares the same.
pub(crate) const DIMENSIONS: usize = 256;

/// A loaded embedding model; cheap to clone.
#[derive(Clone)]
pub(crate) struct Embedder(Inner);

#[derive(Clone)]
enum Inner {
    Model(Arc<StaticModel>),
    /// Hashes each word to a dimension, so texts that share words are close.
    #[cfg(test)]
    Fake,
}

impl fmt::Debug for Embedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Inner::Model(_) => f.write_str("Embedder(Model)"),
            #[cfg(test)]
            Inner::Fake => f.write_str("Embedder(Fake)"),
        }
    }
}

impl Embedder {
    /// Load [`MODEL`], downloading it on first use.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Model`] when the model can't be downloaded or read.
    pub(crate) fn load() -> Result<Self, SearchError> {
        let model = StaticModel::from_pretrained(MODEL, None, None, None)
            .map_err(|err| SearchError::Model(err.into()))?;
        let embedder = Self(Inner::Model(Arc::new(model)));
        // The index stores vectors of a fixed length, so a model that makes others can't be used.
        let length = embedder.embed(&["probe".to_owned()])?.first().map(Vec::len);
        if length != Some(DIMENSIONS) {
            return Err(SearchError::Model(
                format!("{MODEL} makes vectors of {length:?} numbers, not {DIMENSIONS}").into(),
            ));
        }
        Ok(embedder)
    }

    /// The test embedder: words hashed to dimensions, no model needed.
    #[cfg(test)]
    pub(crate) fn fake() -> Self {
        Self(Inner::Fake)
    }

    /// One normalized vector of [`DIMENSIONS`] per text, in order.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::Embed`] when the tokenizer panics; `model2vec-rs` panics instead of
    /// returning tokenizer errors.
    pub(crate) fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, SearchError> {
        match &self.0 {
            Inner::Model(model) => {
                catch_unwind(AssertUnwindSafe(|| model.encode(texts))).map_err(|panic| {
                    let message = panic
                        .downcast_ref::<&str>()
                        .map(ToString::to_string)
                        .or_else(|| panic.downcast_ref::<String>().cloned())
                        .unwrap_or_default();
                    SearchError::Embed(message)
                })
            }
            #[cfg(test)]
            Inner::Fake => Ok(texts.iter().map(|text| fake_vector(text)).collect()),
        }
    }
}

#[cfg(test)]
fn fake_vector(text: &str) -> Vec<f32> {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};

    let mut vector = vec![0.0_f32; DIMENSIONS];
    for word in text.split_whitespace() {
        let mut hasher = DefaultHasher::new();
        word.to_lowercase().hash(&mut hasher);
        let slot = usize::try_from(hasher.finish() % DIMENSIONS as u64).unwrap();
        vector[slot] += 1.0;
    }
    let length = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if length == 0.0 {
        vector[0] = 1.0;
    } else {
        for x in &mut vector {
            *x /= length;
        }
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloads the model, so it only runs on request: `cargo test -p ichnos-search -- --ignored`.
    #[test]
    #[ignore = "downloads the model from Hugging Face"]
    fn the_model_loads_and_ranks_by_meaning() {
        let embedder = Embedder::load().unwrap();
        let texts = [
            "token refresh bug".to_owned(),
            "Updated the OAuth expiry check so expired tokens are renewed.".to_owned(),
            "Listed the files in the docs folder.".to_owned(),
        ];
        let vectors = embedder.embed(&texts).unwrap();
        let distance =
            |a: &[f32], b: &[f32]| 1.0 - a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
        let related = distance(&vectors[0], &vectors[1]);
        let unrelated = distance(&vectors[0], &vectors[2]);
        println!("related {related:.3}, unrelated {unrelated:.3}");
        assert!(related < unrelated);
    }
}
