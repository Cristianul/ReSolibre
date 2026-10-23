//! X11 application identity and task-switcher hints for explicitly registered hosts.

use super::Restacker;
use x11rb::connection::Connection;
use x11rb::properties::WmHints;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub(super) struct Atoms {
    leader: u32,
    state: u32,
    managed: u32,
    skips: [u32; 3],
}

impl Atoms {
    /// Resolves the fixed ICCCM/EWMH names and KDE's explicit Alt+Tab exclusion hint.
    pub(super) fn open(connection: &RustConnection) -> Result<Self, ()> {
        let mut atoms = Vec::with_capacity(6);
        for name in [
            b"WM_CLIENT_LEADER".as_slice(),
            b"_NET_WM_STATE",
            b"_NET_WM_STATE_SKIP_TASKBAR",
            b"_NET_WM_STATE_SKIP_PAGER",
            b"_KDE_NET_WM_STATE_SKIP_SWITCHER",
            b"WM_STATE",
        ] {
            let atom = connection
                .intern_atom(false, name)
                .map_err(|_| ())?
                .reply()
                .map_err(|_| ())?
                .atom;
            atoms.push(atom);
        }
        Ok(Self {
            leader: atoms[0],
            state: atoms[1],
            managed: atoms[5],
            skips: [atoms[2], atoms[3], atoms[4]],
        })
    }
}

impl Restacker {
    /// Preserves input/urgency hints while linking the owned host to its main player.
    fn declare_identity(&self, window: u32, leader: u32) -> Result<(), ()> {
        let mut hints = WmHints::get(&self.connection, window)
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?
            .unwrap_or_default();
        hints.window_group = Some(leader);
        hints
            .set(&self.connection, window)
            .map_err(|_| ())?
            .check()
            .map_err(|_| ())?;
        self.connection
            .change_property32(
                PropMode::REPLACE,
                window,
                self.membership.leader,
                AtomEnum::WINDOW,
                &[leader],
            )
            .map_err(|_| ())?
            .check()
            .map_err(|_| ())?;
        Ok(())
    }

    /// Groups a known live handle; auxiliary hosts have no independent taskbar/switcher entry.
    pub(super) fn configure_member(
        &self,
        window: u32,
        leader: u32,
        declare: bool,
    ) -> Result<bool, ()> {
        if declare {
            self.declare_identity(window, leader)?;
        }
        // The main window keeps all of its existing WM state and remains the entry point.
        if window == leader {
            return Ok(true);
        }
        let reply = self
            .connection
            .get_property(false, window, self.membership.state, AtomEnum::ATOM, 0, 64)
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?;
        if reply.bytes_after != 0 {
            return Err(());
        }
        let current: Vec<u32> = reply.value32().map_or_else(Vec::new, Iterator::collect);
        let managed = self
            .connection
            .get_property(false, window, self.membership.managed, AtomEnum::ANY, 0, 2)
            .map_err(|_| ())?
            .reply()
            .map_err(|_| ())?;
        let managed = managed
            .value32()
            .and_then(|mut states| states.next())
            .is_some_and(|state| state != 0);
        if managed
            && self
                .membership
                .skips
                .iter()
                .all(|atom| current.contains(atom))
        {
            return Ok(true);
        }
        if !managed {
            // Unmapped and minimized are different: only withdrawn hosts accept initial hints.
            let mut states: Vec<u32> = reply.value32().map_or_else(Vec::new, Iterator::collect);
            for atom in self.membership.skips {
                if !states.contains(&atom) {
                    states.push(atom);
                }
            }
            self.connection
                .change_property32(
                    PropMode::REPLACE,
                    window,
                    self.membership.state,
                    AtomEnum::ATOM,
                    &states,
                )
                .map_err(|_| ())?
                .check()
                .map_err(|_| ())?;
        } else {
            for atom in self.membership.skips {
                let event =
                    ClientMessageEvent::new(32, window, self.membership.state, [1, atom, 0, 1, 0]);
                self.connection
                    .send_event(
                        false,
                        self.root,
                        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                        event,
                    )
                    .map_err(|_| ())?
                    .check()
                    .map_err(|_| ())?;
            }
        }
        self.connection.flush().map_err(|_| ())?;
        // Sending is not acknowledgement: the bounded registration timer checks the next reply.
        Ok(false)
    }
}
