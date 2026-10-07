//! The helper's two D-Bus identities.
//!
//! The helper was `net.eterneon.atlas.SystemHelper`; it is now
//! `net.eterneon.telamon.SystemHelper`. For this release one process serves
//! both, from the same implementation, because programs that have not moved
//! yet (older apps, the OS image's own scripts) still call the old one. A
//! call is authorized with the polkit action of the identity it arrived
//! through (`net.eterneon.telamon.system.*` or the legacy
//! `net.eterneon.atlas.system.*`, which an image's polkit rules may refer
//! to), and fails with the error names of that identity.
//!
//! The old identity goes in the release after this one.

/// Which name a call came in through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Identity {
    /// `net.eterneon.telamon.*`
    Telamon,
    /// `net.eterneon.atlas.*`: served for programs that have not moved yet.
    Legacy,
}

impl Identity {
    /// The new identity first: it is the one a client tries first.
    pub const ALL: [Identity; 2] = [Identity::Telamon, Identity::Legacy];

    pub const fn bus_name(self) -> &'static str {
        match self {
            Identity::Telamon => "net.eterneon.telamon.SystemHelper",
            Identity::Legacy => "net.eterneon.atlas.SystemHelper",
        }
    }

    pub const fn object_path(self) -> &'static str {
        match self {
            Identity::Telamon => "/net/eterneon/telamon/SystemHelper",
            Identity::Legacy => "/net/eterneon/atlas/SystemHelper",
        }
    }

    pub const fn interface(self) -> &'static str {
        match self {
            Identity::Telamon => "net.eterneon.telamon.SystemHelper1",
            Identity::Legacy => "net.eterneon.atlas.SystemHelper1",
        }
    }

    /// What error names start with (`<prefix>.Busy`, ...).
    pub const fn error_prefix(self) -> &'static str {
        match self {
            Identity::Telamon => "net.eterneon.telamon.Error",
            Identity::Legacy => "net.eterneon.atlas.Error",
        }
    }

    /// The polkit action ids start with this (`<prefix>.status`, ...).
    pub const fn action_prefix(self) -> &'static str {
        match self {
            Identity::Telamon => "net.eterneon.telamon.system",
            Identity::Legacy => "net.eterneon.atlas.system",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_identities_share_nothing_by_name() {
        let [new, old] = Identity::ALL;
        assert_eq!(new.bus_name(), "net.eterneon.telamon.SystemHelper");
        assert_eq!(old.bus_name(), "net.eterneon.atlas.SystemHelper");
        assert_ne!(new.object_path(), old.object_path());
        assert_ne!(new.interface(), old.interface());
        assert_ne!(new.error_prefix(), old.error_prefix());
        assert_ne!(new.action_prefix(), old.action_prefix());
        // each one's names follow from its bus name
        for id in Identity::ALL {
            assert_eq!(
                id.object_path(),
                format!("/{}", id.bus_name().replace('.', "/"))
            );
            assert_eq!(id.interface(), format!("{}1", id.bus_name()));
        }
    }
}
