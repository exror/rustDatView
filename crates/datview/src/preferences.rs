use super::*;

const STORAGE_KEY: &str = "startup_preferences";

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Preset {
    name: String,
    settings: Options,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct Preferences {
    restore_previous: bool,
    default_preset: Option<String>,
    presets: Vec<Preset>,
    last_settings: Option<Options>,
    draft: Option<Options>,
}

fn same_settings(a: &Options, b: &Options) -> bool {
    a == b
}

impl Preferences {
    pub fn load(storage: Option<&dyn eframe::Storage>) -> Self {
        let Some(storage) = storage else {
            return Self::default();
        };
        if let Some(preferences) = eframe::get_value(storage, STORAGE_KEY) {
            return preferences;
        }
        // One-time migration: old implicit persistence becomes a recoverable draft,
        // never an opt-in to restoring settings or data-processing operations.
        let mut preferences = Self::default();
        if let Some(previous) = eframe::get_value::<Options>(storage, "options") {
            preferences.record(previous);
        }
        preferences
    }

    pub fn startup(&self) -> Options {
        if self.restore_previous {
            if let Some(previous) = &self.last_settings {
                return previous.clone();
            }
        }
        self.default_preset
            .as_ref()
            .and_then(|name| self.presets.iter().find(|p| &p.name == name))
            .map(|p| p.settings.clone())
            .unwrap_or_default()
    }

    pub fn record(&mut self, settings: Options) {
        if !same_settings(&settings, &Options::default())
            && !self
                .presets
                .iter()
                .any(|p| same_settings(&settings, &p.settings))
        {
            self.draft = Some(settings.clone());
        }
        self.last_settings = Some(settings);
    }

    pub fn store(&self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, STORAGE_KEY, self);
        // Stop writing the obsolete key; erase its old payload after migration.
        storage.set_string("options", String::new());
    }

    fn clear_matching_draft(&mut self, settings: &Options) {
        if self
            .draft
            .as_ref()
            .is_some_and(|draft| same_settings(draft, settings))
        {
            self.draft = None;
        }
    }

    fn save_new(
        &mut self,
        name: &str,
        settings: Options,
        make_default: bool,
    ) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a preset name.".into());
        }
        if self
            .presets
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(name))
        {
            return Err("That preset name already exists. Select it and use Update selected, or choose another name.".into());
        }
        self.clear_matching_draft(&settings);
        self.presets.push(Preset {
            name: name.into(),
            settings,
        });
        if make_default {
            self.default_preset = Some(name.into());
        }
        Ok(name.into())
    }

    fn update(&mut self, name: &str, settings: Options) -> Result<(), String> {
        let index = self
            .presets
            .iter()
            .position(|p| p.name == name)
            .ok_or("Preset no longer exists.")?;
        self.clear_matching_draft(&settings);
        self.presets[index].settings = settings;
        Ok(())
    }

    fn remove(&mut self, name: &str) {
        self.presets.retain(|p| p.name != name);
        if self.default_preset.as_deref() == Some(name) {
            self.default_preset = None;
        }
    }
}

pub(super) struct PresetUi {
    name: String,
    selected: Option<String>,
    make_default: bool,
    recovery: Option<Options>,
    show_recovery: bool,
    recovery_name: String,
    recovery_default: bool,
}

impl PresetUi {
    pub fn new(preferences: &Preferences) -> Self {
        let recovery = preferences.draft.clone();
        let mut recovery_name = "Recovered settings".to_owned();
        let mut suffix = 2;
        while preferences
            .presets
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(&recovery_name))
        {
            recovery_name = format!("Recovered settings {suffix}");
            suffix += 1;
        }
        Self {
            name: String::new(),
            selected: None,
            make_default: false,
            show_recovery: recovery.is_some() && !preferences.restore_previous,
            recovery,
            recovery_name,
            recovery_default: false,
        }
    }
}

impl DatView {
    pub(super) fn settings_snapshot(&self) -> Options {
        let mut settings = self.options.clone();
        settings.plot_options = serde_json::to_value(&self.plot_type).unwrap_or(Value::Null);
        settings
    }

    pub(super) fn apply_settings(&mut self, mut settings: Options, ctx: &egui::Context) {
        let plot_type = match serde_json::from_value(settings.plot_options.clone()) {
            Ok(value) => value,
            Err(error) => {
                self.error(format!("Cannot apply preset: {error}"));
                return;
            }
        };
        if let Err(error) = self.appearance.apply_font(ctx, &settings.style.font_family) {
            settings.style.font_family.clear();
            let _ = self.appearance.apply_font(ctx, "");
            self.error(error);
        }
        appearance::apply_theme(ctx, settings.dark, settings.font_size);
        self.options = settings;
        self.plot_type = plot_type;
        self.tables.dayfirst = self.options.dayfirst;
        self.limits_text = self
            .options
            .style
            .limits
            .map(|b| {
                b.iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        self.overlays.clear();
        self.measurements.clear();
        self.presentation.pending = None;
        self.presentation.tool = presentation::Tool::Inspect;
        self.field_texture = None;
        self.field_raster = None;
        // A preset's explicit limits win over cached interactive zoom/pan bounds.
        ctx.memory_mut(|memory| memory.data.clear());
        self.dirty = true;
    }

    pub(super) fn startup_settings(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.group(|ui| {
            ui.strong("Startup & presets");
            ui.weak("Start fresh unless you explicitly choose otherwise.");
            ui.checkbox(&mut self.preferences.restore_previous, "Restore previous settings on startup");
            ui.horizontal(|ui| {
                ui.label("Default preset");
                egui::ComboBox::from_id_salt("startup-default-preset")
                    .selected_text(self.preferences.default_preset.as_deref().unwrap_or("Factory defaults"))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.preferences.default_preset, None, "Factory defaults");
                        for preset in &self.preferences.presets {
                            ui.selectable_value(&mut self.preferences.default_preset, Some(preset.name.clone()), &preset.name);
                        }
                    });
            });
            if self.preferences.restore_previous {
                ui.small("Previous settings take precedence; the default preset is the fallback.");
            } else {
                ui.small("The default preset is copied at startup. Later edits never overwrite it.");
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Saved presets");
                egui::ComboBox::from_id_salt("saved-settings-preset")
                    .selected_text(self.preset_ui.selected.as_deref().unwrap_or("Select a preset"))
                    .show_ui(ui, |ui| {
                        for preset in &self.preferences.presets {
                            ui.selectable_value(&mut self.preset_ui.selected, Some(preset.name.clone()), &preset.name);
                        }
                    });
            });
            let selected = self.preset_ui.selected.clone();
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(selected.is_some(), egui::Button::new("Apply selected")).clicked() {
                    if let Some(preset) = self.preferences.presets.iter().find(|p| Some(&p.name) == selected.as_ref()) {
                        let settings = preset.settings.clone();
                        self.apply_settings(settings, ctx);
                        self.status = "Preset applied to settings. Loaded data and pipeline unchanged.".into();
                    }
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Update selected")).clicked() {
                    if let Some(name) = &selected {
                        let settings = self.settings_snapshot();
                        match self.preferences.update(name, settings) { Ok(()) => self.status = format!("Updated preset: {name}"), Err(error) => self.error(error) }
                    }
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Delete selected")).clicked() {
                    if let Some(name) = &selected { self.preferences.remove(name); self.preset_ui.selected = None; }
                }
            });
            ui.horizontal(|ui| {
                ui.label("New preset");
                ui.add(egui::TextEdit::singleline(&mut self.preset_ui.name).hint_text("e.g. Motherboard power").desired_width(230.));
            });
            ui.checkbox(&mut self.preset_ui.make_default, "Use new preset as startup default");
            if ui.button("Save current settings as new preset").clicked() {
                let settings = self.settings_snapshot();
                match self.preferences.save_new(&self.preset_ui.name, settings, self.preset_ui.make_default) {
                    Ok(name) => { self.status = format!("Saved preset: {name}"); self.preset_ui.selected = Some(name); self.preset_ui.name.clear(); self.preset_ui.make_default = false; },
                    Err(error) => self.error(error),
                }
            }
            if self.preferences.draft.is_some() && ui.button("Review autosaved settings draft…").clicked() {
                self.preset_ui.recovery = self.preferences.draft.clone(); self.preset_ui.show_recovery = true;
            }
            ui.small("Presets contain appearance, plot/field options and display preferences, not files, data, pipeline actions, annotations or display filters. Use Save pipeline separately for processing operations.");
        });
        ui.add_space(6.);
    }

    pub(super) fn recovery_window(&mut self, ctx: &egui::Context) {
        if !self.preset_ui.show_recovery {
            return;
        }
        let Some(recovery) = self.preset_ui.recovery.take() else {
            self.preset_ui.show_recovery = false;
            return;
        };
        let mut keep_recovery = true;
        let mut open = true;
        egui::Window::new("Keep previous settings as a preset?")
            .id(egui::Id::new("settings-draft-recovery"))
            .default_width(430.)
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(
                    "An autosaved settings draft is available. Reviewing or saving it does not change the current view.",
                );
                ui.weak("Save them as a named preset, discard them, or decide later.");
                ui.add_space(6.);
                ui.label(format!("Plot mode: {}", recovery.mode));
                if !recovery.style.title.is_empty() {
                    ui.label(format!("Figure title: {}", recovery.style.title));
                }
                ui.horizontal(|ui| {
                    ui.label("Preset name");
                    ui.text_edit_singleline(&mut self.preset_ui.recovery_name);
                });
                ui.checkbox(
                    &mut self.preset_ui.recovery_default,
                    "Use as default on future launches",
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.button("Save preset").clicked() {
                        match self.preferences.save_new(
                            &self.preset_ui.recovery_name,
                            recovery.clone(),
                            self.preset_ui.recovery_default,
                        ) {
                            Ok(name) => {
                                self.status =
                                    format!("Saved preset: {name}. Current view unchanged.");
                                self.preset_ui.selected = Some(name);
                                self.preset_ui.show_recovery = false;
                                keep_recovery = false;
                            }
                            Err(error) => self.error(error),
                        }
                    }
                    if ui.button("Discard draft").clicked() {
                        self.preferences.clear_matching_draft(&recovery);
                        keep_recovery = false;
                        self.preset_ui.show_recovery = false;
                    }
                    if ui.button("Later").clicked() {
                        self.preset_ui.show_recovery = false;
                    }
                });
            });
        if !open {
            self.preset_ui.show_recovery = false;
        }
        if keep_recovery {
            self.preset_ui.recovery = Some(recovery);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Storage(BTreeMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    fn settings(title: &str) -> Options {
        let mut options = Options::default();
        options.style.title = title.into();
        options.dark = true;
        options
    }
    #[test]
    fn legacy_settings_become_a_draft_not_startup_configuration() {
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, "options", &settings("old"));
        let preferences = Preferences::load(Some(&storage));
        assert!(preferences.startup().style.title.is_empty());
        assert!(!preferences.startup().dark);
        assert_eq!(preferences.draft.as_ref().unwrap().style.title, "old");
        preferences.store(&mut storage);
        assert!(eframe::get_value::<Options>(&storage, "options").is_none());
        assert!(Preferences::load(Some(&storage))
            .startup()
            .style
            .title
            .is_empty());
    }
    #[test]
    fn edits_do_not_overwrite_a_default_preset_across_restarts() {
        let mut preferences = Preferences::default();
        preferences
            .save_new("Power", settings("baseline"), true)
            .unwrap();
        preferences.record(settings("one-off edit"));
        let mut storage = Storage::default();
        preferences.store(&mut storage);
        let mut next = Preferences::load(Some(&storage));
        assert_eq!(next.startup().style.title, "baseline");
        assert_eq!(next.draft.as_ref().unwrap().style.title, "one-off edit");
        next.record(next.startup());
        next.store(&mut storage);
        assert_eq!(
            Preferences::load(Some(&storage)).draft.unwrap().style.title,
            "one-off edit"
        );
    }
    #[test]
    fn restoring_previous_settings_is_explicit_and_has_defined_precedence() {
        let mut preferences = Preferences::default();
        preferences
            .save_new("Power", settings("baseline"), true)
            .unwrap();
        preferences.restore_previous = true;
        assert_eq!(preferences.startup().style.title, "baseline");
        preferences.record(settings("latest"));
        assert_eq!(preferences.startup().style.title, "latest");
        assert!(!PresetUi::new(&preferences).show_recovery);
        preferences.restore_previous = false;
        assert_eq!(preferences.startup().style.title, "baseline");
        assert!(PresetUi::new(&preferences).show_recovery);
    }
    #[test]
    fn saving_recovery_does_not_apply_it_or_change_the_default_unless_requested() {
        let mut preferences = Preferences::default();
        preferences.record(settings("draft"));
        preferences
            .save_new("Recovered", preferences.draft.clone().unwrap(), false)
            .unwrap();
        assert!(preferences.startup().style.title.is_empty());
        assert!(preferences.draft.is_none());
        preferences.record(settings("other"));
        preferences.clear_matching_draft(&settings("draft"));
        assert_eq!(preferences.draft.as_ref().unwrap().style.title, "other");
        preferences.clear_matching_draft(&settings("other"));
        let mut storage = Storage::default();
        preferences.store(&mut storage);
        assert!(Preferences::load(Some(&storage)).draft.is_none());
    }
    #[test]
    fn preset_updates_and_removal_require_explicit_selection() {
        let mut preferences = Preferences::default();
        preferences
            .save_new("Power", settings("first"), true)
            .unwrap();
        assert!(preferences
            .save_new(" power ", settings("second"), false)
            .is_err());
        assert!(preferences.update("missing", settings("second")).is_err());
        assert_eq!(preferences.startup().style.title, "first");
        preferences.update("Power", settings("second")).unwrap();
        assert_eq!(preferences.startup().style.title, "second");
        preferences.remove("Power");
        assert!(preferences.default_preset.is_none());
        assert!(preferences.startup().style.title.is_empty());
    }
}
