//! Turns the nearest docs into one result per session.

use ichnos_core::store::SessionId;

use crate::docs::DocKind;

/// The largest cosine distance still shown as a match. On real sessions with `potion-base-8M`,
/// related text measured 0.19–0.56 and unrelated text above 0.75.
pub(crate) const MAX_DISTANCE: f32 = 0.6;

/// A session that matched a search, with the text that matched best.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMatch {
    /// The session the text is from.
    pub session: SessionId,
    /// The session's `events.id` the text belongs to.
    pub event_id: i64,
    /// Whether the text is a prompt, an error or a summary.
    pub kind: DocKind,
    /// The text that matched.
    pub text: String,
    /// Cosine distance to the search: 0 is the same meaning, 2 the opposite.
    pub distance: f32,
}

/// The closest doc of each session in `nearest` (closest first), without docs farther than
/// [`MAX_DISTANCE`].
pub(crate) fn rank(nearest: Vec<SessionMatch>) -> Vec<SessionMatch> {
    let mut ranked: Vec<SessionMatch> = Vec::new();
    for found in nearest {
        if found.distance <= MAX_DISTANCE
            && !ranked.iter().any(|kept| kept.session == found.session)
        {
            ranked.push(found);
        }
    }
    ranked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(session: &str, text: &str, distance: f32) -> SessionMatch {
        SessionMatch {
            session: session.parse().unwrap(),
            event_id: 1,
            kind: DocKind::Summary,
            text: text.to_owned(),
            distance,
        }
    }

    #[test]
    fn each_session_keeps_its_closest_doc_in_order() {
        let ranked = rank(vec![
            found("a", "a1", 0.1),
            found("b", "b1", 0.2),
            found("a", "a2", 0.3),
        ]);
        let texts: Vec<&str> = ranked.iter().map(|found| found.text.as_str()).collect();
        assert_eq!(texts, ["a1", "b1"]);
    }

    #[test]
    fn docs_farther_than_the_cutoff_are_dropped() {
        assert_eq!(rank(vec![found("a", "far", MAX_DISTANCE + 0.01)]), []);
    }
}
