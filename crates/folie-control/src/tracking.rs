//! Feuille de tracking : pour chaque paramètre et chaque cue d'une séquence,
//! la valeur tenue et son origine (changée ici, suivie depuis une cue
//! précédente, relâchée), comme la Tracking Sheet de grandMA2.

use std::collections::{BTreeMap, BTreeSet};

use folie_fixture::FixtureId;
use folie_show::{CueNumber, CueValue};

use crate::Console;

/// Ce que montre une case de la feuille.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TrackedValue {
    /// Rien n'est encore tenu, ou le paramètre a été relâché plus tôt.
    Empty,
    /// Valeur (ou référence à un preset), venue de la cue `origin` (index dans la séquence).
    Held { value: CueValue, origin: usize },
    /// Relâché dans cette cue.
    Released,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrackingCell {
    pub value: TrackedValue,
    /// Animé par un effet de la séquence dans cette cue.
    pub effect: bool,
    /// (fondu, délai) propres au paramètre ou à sa famille dans cette cue, s'il en a.
    pub own_times: Option<(f32, f32)>,
}

impl TrackingCell {
    /// La case tient une valeur (et non un vide ou un relâchement).
    pub fn holds_value(&self) -> bool {
        matches!(self.value, TrackedValue::Held { .. })
    }

    /// Valeur enregistrée dans cette cue même (et non suivie).
    pub fn changed_here(&self, cue: usize) -> bool {
        match self.value {
            TrackedValue::Held { origin, .. } => origin == cue,
            TrackedValue::Released => true,
            TrackedValue::Empty => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrackingRow {
    pub fixture: FixtureId,
    pub fixture_name: String,
    pub parameter: String,
    /// Une case par cue, dans l'ordre de la séquence.
    pub cells: Vec<TrackingCell>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackingSheet {
    pub cues: Vec<(CueNumber, String)>,
    /// Paramètres touchés par la séquence, fixture par fixture, dans l'ordre du patch.
    pub rows: Vec<TrackingRow>,
}

impl Console {
    /// Feuille de tracking de la séquence d'un executor ; `None` s'il est vide.
    pub fn tracking_sheet(&self, executor: u32) -> Option<TrackingSheet> {
        let (_, sequence) = self.executor(executor)?;
        let patch = &self.show.patch;
        let cues: Vec<(CueNumber, String)> = sequence.cues.iter().map(|(n, c)| (*n, c.name.clone())).collect();

        // Paramètres animés par les effets en cours dans chaque cue (les effets se suivent aussi).
        let mut animated: Vec<BTreeSet<(FixtureId, String)>> = Vec::new();
        let mut current = BTreeSet::new();
        for cue in sequence.cues.values() {
            if let Some(effects) = &cue.effects {
                current = effects
                    .iter()
                    .flat_map(|e| crate::effects::instance(e, 0, patch).lines)
                    .flat_map(|line| line.params.into_iter().map(|(key, _)| key))
                    .filter_map(|key| {
                        let fixture = patch.get(key.fixture)?;
                        Some((key.fixture, fixture.mode().parameters.get(key.parameter)?.name.clone()))
                    })
                    .collect();
            }
            animated.push(current.clone());
        }

        // Parcours des cues : la dernière valeur de chaque paramètre et la cue d'où elle vient.
        let mut held: BTreeMap<(FixtureId, String), TrackedValue> = BTreeMap::new();
        let mut columns: Vec<BTreeMap<(FixtureId, String), TrackedValue>> = Vec::new();
        for (index, cue) in sequence.cues.values().enumerate() {
            // Un relâchement ne dure qu'une cue : ensuite, la case est vide.
            held.retain(|_, v| *v != TrackedValue::Released);
            for (fixture, parameters) in &cue.values {
                for (name, value) in parameters {
                    let tracked = match value {
                        CueValue::Release => TrackedValue::Released,
                        value => TrackedValue::Held { value: *value, origin: index },
                    };
                    held.insert((*fixture, name.clone()), tracked);
                }
            }
            columns.push(held.clone());
        }

        let touched: BTreeSet<(FixtureId, String)> =
            columns.iter().flat_map(|c| c.keys().cloned()).chain(animated.iter().flatten().cloned()).collect();
        let sequence_cues: Vec<_> = sequence.cues.values().collect();
        let mut rows = Vec::new();
        for fixture in patch.fixtures() {
            for parameter in &fixture.mode().parameters {
                let key = (fixture.id, parameter.name.clone());
                if !touched.contains(&key) {
                    continue;
                }
                let family = parameter.attribute.group();
                let cells = (0..cues.len())
                    .map(|i| {
                        let cue = sequence_cues[i];
                        let own = cue.has_own_times(fixture.id, &parameter.name, family);
                        TrackingCell {
                            value: columns[i].get(&key).copied().unwrap_or(TrackedValue::Empty),
                            effect: animated[i].contains(&key),
                            own_times: own.then(|| cue.times_of(fixture.id, &parameter.name, family)),
                        }
                    })
                    .collect();
                rows.push(TrackingRow {
                    fixture: fixture.id,
                    fixture_name: fixture.name.clone(),
                    parameter: parameter.name.clone(),
                    cells,
                });
            }
        }
        Some(TrackingSheet { cues, rows })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{console, run};
    use folie_fixture::FeatureGroup;
    use folie_show::ParamTiming;

    #[test]
    fn values_are_tracked_released_and_animated() {
        let mut c = console();
        run(&mut c, "Fixture 1 At 100");
        run(&mut c, "Store Cue 1");
        run(&mut c, "ClearAll");
        run(&mut c, "Fixture 2 At 50");
        run(&mut c, "Store Cue 2");
        run(&mut c, "ClearAll");
        run(&mut c, "Fixture 1 At 0");
        run(&mut c, "Effect 3");
        run(&mut c, "Store Cue 3");
        run(&mut c, "ClearAll");

        let sheet = c.tracking_sheet(1).unwrap();
        assert_eq!(sheet.cues.len(), 3);
        let row = |fixture: u32, parameter: &str| {
            sheet.rows.iter().find(|r| r.fixture == FixtureId(fixture) && r.parameter == parameter).unwrap()
        };
        let dimmer1 = row(1, "Dimmer");
        assert_eq!(dimmer1.cells[0].value, TrackedValue::Held { value: CueValue::Value(65_535), origin: 0 });
        assert!(dimmer1.cells[0].changed_here(0));
        assert_eq!(dimmer1.cells[1].value, TrackedValue::Held { value: CueValue::Value(65_535), origin: 0 }, "suivie");
        assert!(!dimmer1.cells[1].changed_here(1));
        assert_eq!(dimmer1.cells[2].value, TrackedValue::Held { value: CueValue::Value(0), origin: 2 });

        let dimmer2 = row(2, "Dimmer");
        assert_eq!(dimmer2.cells[0].value, TrackedValue::Empty, "pas encore dans la séquence");
        assert!(dimmer2.cells[2].value != TrackedValue::Empty);
        assert!(sheet.rows.iter().any(|r| r.cells[2].effect), "l'effet de la cue 3 anime des paramètres");
        assert!(sheet.rows.iter().all(|r| !r.cells[0].effect));

        // Release : relâché dans sa cue, vide ensuite.
        let release = |c: &mut Console| {
            let sequence = c.show.executors[&1].sequence;
            let cue = c.show.sequences.get_mut(&sequence).unwrap().cues.values_mut().nth(1).unwrap();
            cue.values.entry(FixtureId(1)).or_default().insert("Dimmer".into(), CueValue::Release);
        };
        release(&mut c);
        let sheet = c.tracking_sheet(1).unwrap();
        let dimmer1 = sheet.rows.iter().find(|r| r.fixture == FixtureId(1) && r.parameter == "Dimmer").unwrap();
        assert_eq!(dimmer1.cells[1].value, TrackedValue::Released);
        assert_eq!(dimmer1.cells[2].value, TrackedValue::Held { value: CueValue::Value(0), origin: 2 });
        assert!(c.tracking_sheet(9).is_none(), "executor vide");

        // Temps propres : la famille Dimmer en cue 3, puis le Dimmer de la fixture 1 seul.
        let third = c.show.sequences[&1].cues.keys().nth(2).copied().unwrap();
        let four = ParamTiming { fade: Some(4.0), delay: None };
        c.set_family_times(third, FeatureGroup::Dimmer, four).unwrap();
        c.set_param_times(third, FixtureId(1), "Dimmer", ParamTiming { fade: Some(6.0), delay: Some(1.0) }).unwrap();
        let sheet = c.tracking_sheet(1).unwrap();
        let cell = |fixture: u32| {
            sheet.rows.iter().find(|r| r.fixture == FixtureId(fixture) && r.parameter == "Dimmer").unwrap().cells[2]
        };
        assert_eq!(cell(1).own_times, Some((6.0, 1.0)), "le paramètre l'emporte sur sa famille");
        assert_eq!(cell(2).own_times, Some((4.0, 0.0)), "temps de la famille");
        // Le moteur reçoit les temps propres des paramètres de la cue.
        let played = crate::cues::playback_data(&c.show);
        let dimmer1 = folie_fixture::ParamKey { fixture: FixtureId(1), parameter: 1 };
        assert_eq!(played[&1][2].times.get(&dimmer1), Some(&(6.0, 1.0)));
        assert!(played[&1][1].times.is_empty(), "rien de propre dans la cue 2");
        c.set_param_times(third, FixtureId(1), "Dimmer", ParamTiming::default()).unwrap();
        assert!(c.show.sequences[&1].cues[&third].param_times.is_empty(), "temps vides : retirés");
    }
}
