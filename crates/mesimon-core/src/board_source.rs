//! Generic capability floor for board clients. This is not an identity, grant,
//! transport, or a substitute for authorization at the state writer.

/// A repository source has a real local binding, established by its caller.
/// A content-only source has no checkout, daemon, session, or execution host.
/// Neither variant carries a fabricated path or serialized authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BoardSource {
    Repository,
    #[default]
    ContentOnly,
}

/// Keep content operations separate from every capability requiring a host.
/// Supporting a capability says it is meaningful on a source, not permitted
/// for the current principal. Authorization and per-ticket policy still apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardCapability {
    ReadContent,
    EditContent,
    Repository,
    Sessions,
    Execution,
}

impl BoardSource {
    pub fn supports(self, capability: BoardCapability) -> bool {
        match (self, capability) {
            (_, BoardCapability::ReadContent | BoardCapability::EditContent) => true,
            (Self::Repository, _) => true,
            (Self::ContentOnly, _) => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_repository_means_no_host_capabilities() {
        for source in [BoardSource::ContentOnly, BoardSource::default()] {
            assert!(source.supports(BoardCapability::ReadContent));
            assert!(source.supports(BoardCapability::EditContent));
            for capability in
                [BoardCapability::Repository, BoardCapability::Sessions, BoardCapability::Execution]
            {
                assert!(!source.supports(capability));
                assert!(BoardSource::Repository.supports(capability));
            }
        }
    }
}
