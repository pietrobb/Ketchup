//! How the viewport presents the model: independent on/off switches that
//! change painting only, never the document or what can be picked.

/// One presentation switch. `AppCommand::View(flag)` toggles it; its label
/// is the command's `view-<name>` key and the toggle reports
/// `digest-<name>-shown` / `digest-<name>-hidden`.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ViewFlag {
    /// The adaptive construction grid and world axes.
    GridAxes,
    /// A clean white presentation background.
    WhiteBackground,
    /// Deterministic ground shadows of visible model bounds.
    Shadows,
    /// Depth haze over the scene.
    Fog,
    /// Hidden occurrences outlined as non-interactive ghosts.
    HiddenObjects,
    /// Translucent geometry.
    Xray,
    /// Edges without face fills.
    Wireframe,
    /// Face colors as neutral grayscale.
    Monochrome,
    /// Faces in one flat neutral color.
    HiddenLine,
    /// Feature-edge outlines.
    Edges,
    /// Emphasized model profiles.
    Profiles,
    /// A fixed screen-space background under-stroke below edges.
    Halos,
    /// Edge strokes weighted by camera depth.
    DepthCue,
    /// Distant edges fading toward the background.
    FadeDistantEdges,
    /// Edges in maximum contrast against the background.
    HighContrastEdges,
    /// A background-aware under-stroke below selected edges.
    SelectionHalo,
    /// Edge endpoint markers.
    Endpoints,
    /// Edge midpoint markers.
    Midpoints,
    /// Fixed screen-space extensions at both edge ends.
    Extensions,
    /// Screen-space edge jitter.
    Jitter,
    /// A fixed screen-space dash rhythm on edges.
    Dashes,
    /// Edges in their dominant world-axis color.
    ColorByAxis,
}

impl ViewFlag {
    /// Every switch, in menu order.
    pub const ALL: [Self; 22] = [
        Self::GridAxes,
        Self::WhiteBackground,
        Self::Shadows,
        Self::Fog,
        Self::HiddenObjects,
        Self::Xray,
        Self::Wireframe,
        Self::Monochrome,
        Self::HiddenLine,
        Self::Edges,
        Self::Profiles,
        Self::Halos,
        Self::DepthCue,
        Self::FadeDistantEdges,
        Self::HighContrastEdges,
        Self::SelectionHalo,
        Self::Endpoints,
        Self::Midpoints,
        Self::Extensions,
        Self::Jitter,
        Self::Dashes,
        Self::ColorByAxis,
    ];

    const fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// The set of switches that are on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewSettings {
    on: u32,
}

impl Default for ViewSettings {
    /// Grid, axes and edges on; everything else off.
    fn default() -> Self {
        Self {
            on: ViewFlag::GridAxes.bit() | ViewFlag::Edges.bit(),
        }
    }
}

impl ViewSettings {
    #[must_use]
    pub const fn contains(self, flag: ViewFlag) -> bool {
        self.on & flag.bit() != 0
    }

    pub fn set(&mut self, flag: ViewFlag, on: bool) {
        if on {
            self.on |= flag.bit();
        } else {
            self.on &= !flag.bit();
        }
    }

    /// Flips `flag` and returns whether it is now on.
    pub fn toggle(&mut self, flag: ViewFlag) -> bool {
        self.on ^= flag.bit();
        self.contains(flag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_are_independent_bits() {
        let mut view = ViewSettings::default();
        let on: Vec<_> = ViewFlag::ALL
            .into_iter()
            .filter(|flag| view.contains(*flag))
            .collect();
        assert_eq!(on, [ViewFlag::GridAxes, ViewFlag::Edges]);
        for flag in ViewFlag::ALL {
            let before = view;
            assert_eq!(view.toggle(flag), !before.contains(flag));
            for other in ViewFlag::ALL.into_iter().filter(|other| *other != flag) {
                assert_eq!(
                    view.contains(other),
                    before.contains(other),
                    "{flag:?} moved {other:?}"
                );
            }
            view.set(flag, before.contains(flag));
            assert_eq!(view, before);
        }
    }
}
