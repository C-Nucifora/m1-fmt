use m1_core::Kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceDecision {
    None,
    Single,
}

/// Returns the spacing to emit between `prev` and `next` given their parent context.
pub fn space_between(_prev: Kind, _next: Kind, _parent: Kind) -> SpaceDecision {
    SpaceDecision::Single // placeholder
}
