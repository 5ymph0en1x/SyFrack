//! Export workspace: settings, live output summary and contextual job controls.
use super::*;
use crate::ui::{file_browser, floating};
use egui::{Align, FontId, Layout, Stroke, Vec2};

pub(super) fn directory_result_id() -> egui::Id {
    egui::Id::new("render-directory-result")
}

impl RenderPanel {
    pub(super) fn window(&mut self, ui: &mut Ui, state: &AppState) -> Action {
        let mut action = Action::None;
        self.size_active = false;
        if let Some(path) = ui.data_mut(|d| d.remove_temp::<PathBuf>(directory_result_id())) {
            self.settings.dir = path.display().to_string();
        }
        if !self.open {
            return action;
        }
        self.sync_scene_quality(&state.scene);
        let mut open = true;
        let mut hide = false;
        floating::Window::new(syfrack_i18n::tr!("Rendre / Exporter"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_pos(egui::pos2(40.0, 40.0))
            .default_width(PANEL_W)
            .max_height((ui.ctx().content_rect().height() - 100.0).max(320.0))
            .show(ui.ctx(), |ui| {
                let desktop = floating::is_secondary(ui.ctx());
                if !desktop {
                    ui.set_width(PANEL_W.min(ui.ctx().content_rect().width() - 100.0));
                }
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                self.header(ui, state);
                ui.add_space(16.0);
                let bottom = if desktop {
                    ui.ctx().content_rect().bottom() - 20.0
                } else {
                    ui.ctx().content_rect().bottom() - 56.0
                };
                let content_height = (bottom - ui.cursor().min.y - 88.0).max(160.0);
                egui::ScrollArea::vertical()
                    .id_salt("render-content")
                    .max_height(content_height)
                    .auto_shrink([false, !desktop])
                    .show(ui, |ui| {
                        let wide = ui.available_width() >= 820.0;
                        let busy = self.job.is_some();
                        if wide {
                            ui.horizontal_top(|ui| {
                                let left = ui.available_width() - 292.0;
                                ui.allocate_ui_with_layout(
                                    Vec2::new(left, 0.0),
                                    Layout::top_down(Align::Min),
                                    |ui| {
                                        ui.set_min_width(left);
                                        self.main_column(
                                            ui,
                                            busy,
                                            self.settings.dimension
                                                == syfrack_core::Dimension::TwoD,
                                        );
                                    },
                                );
                                ui.add_space(8.0);
                                ui.vertical(|ui| {
                                    ui.set_width(ui.available_width());
                                    self.summary(ui, state, busy);
                                });
                            });
                        } else {
                            self.main_column(
                                ui,
                                busy,
                                self.settings.dimension == syfrack_core::Dimension::TwoD,
                            );
                            ui.add_space(12.0);
                            card(ui, |ui| {
                                ui.add_enabled_ui(!busy, |ui| {
                                    self.destination_form(ui, state, false)
                                });
                            });
                        }
                    });
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(10.0);
                action = self.footer(ui, &mut hide);
            });
        self.open = open && !hide;
        action
    }

    fn header(&self, ui: &mut Ui, state: &AppState) {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                Vec2::new(ui.available_width() - 170.0, 0.0),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.label(
                        RichText::new(syfrack_i18n::tr!("Rendu final"))
                            .size(25.0)
                            .strong(),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(if self.job.is_some() {
                                self.stem
                                    .as_deref()
                                    .and_then(|s| s.file_name())
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| default_name(state.path.as_deref()))
                            } else {
                                default_name(state.path.as_deref())
                            })
                            .color(theme::TEXT_DIM),
                        )
                        .truncate(),
                    );
                },
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, color) = match self.status {
                    Status::Idle => (syfrack_i18n::tr!("Préparation"), theme::ACCENT),
                    Status::Running => (syfrack_i18n::tr!("Calcul en cours"), theme::ACCENT),
                    Status::Pausing => (syfrack_i18n::tr!("Mise en pause"), theme::TEXT_DIM),
                    Status::Paused => (syfrack_i18n::tr!("En pause"), theme::TEXT_DIM),
                    Status::Cancelled => (syfrack_i18n::tr!("Interrompu"), theme::TEXT_DIM),
                    Status::Done(_) => (syfrack_i18n::tr!("Terminé"), theme::ACCENT),
                    Status::Failed(_) => (syfrack_i18n::tr!("À vérifier"), theme::ERROR),
                };
                egui::Frame::NONE
                    .fill(theme::FIELD)
                    .corner_radius(12.0)
                    .inner_margin(egui::Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.label(RichText::new(label).small().color(color));
                    });
            });
        });
    }

    fn settings_form(&mut self, ui: &mut Ui, flat: bool) {
        let mut size_hot = false;
        let s = &mut self.settings;
        card(ui, |ui| {
            section(
                ui,
                "Image",
                syfrack_i18n::tr!("Définition et orientation du fichier final."),
            );
            let width =
                (ui.available_width() - 8.0 * (PRESETS.len() - 1) as f32) / PRESETS.len() as f32;
            ui.horizontal(|ui| {
                for (label, w, h) in PRESETS {
                    let (w, h) = if s.height > s.width { (h, w) } else { (w, h) };
                    let response = preset(
                        ui,
                        width,
                        label,
                        &format!("{w} × {h}"),
                        (s.width, s.height) == (w, h),
                    );
                    #[cfg(test)]
                    ui.data_mut(|d| {
                        d.insert_temp(egui::Id::new(("render-size", label)), response.rect)
                    });
                    size_hot |= response.hovered() || response.has_focus();
                    if response.clicked() {
                        (s.width, s.height) = (w, h);
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 112.0) / 2.0;
                for (label, value) in [
                    (syfrack_i18n::tr!("Largeur"), &mut s.width),
                    (syfrack_i18n::tr!("Hauteur"), &mut s.height),
                ] {
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        caption(ui, label);
                        let r = ui.add_sized(
                            [width, 32.0],
                            egui::DragValue::new(value)
                                .range(16..=16384)
                                .speed(8)
                                .suffix(" px"),
                        );
                        size_hot |= r.hovered() || r.has_focus() || r.dragged();
                    });
                }
                ui.vertical(|ui| {
                    caption(ui, "Orientation");
                    let swap = ui
                        .add_sized(
                            [96.0, 32.0],
                            egui::Button::new(syfrack_i18n::tr!("Inverser")),
                        )
                        .on_hover_text(syfrack_i18n::tr!("Permuter la largeur et la hauteur"));
                    size_hot |= swap.hovered() || swap.has_focus();
                    if swap.clicked() {
                        std::mem::swap(&mut s.width, &mut s.height);
                    }
                });
            });
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            section(
                ui,
                syfrack_i18n::tr!("Qualité"),
                syfrack_i18n::tr!("Choisir le niveau de calcul, puis affiner si nécessaire."),
            );
            let width = (ui.available_width() - 16.0) / 3.0;
            let flame = s.flame.is_some();
            if flat {
                let choices = if flame {
                    [
                        (syfrack_i18n::tr!("Étude"), 32),
                        ("Production", 128),
                        ("Ultra", 512),
                    ]
                } else {
                    [
                        (syfrack_i18n::tr!("Rapide"), 8),
                        (syfrack_i18n::tr!("Soigné"), 16),
                        (syfrack_i18n::tr!("Très fin"), 32),
                    ]
                };
                ui.horizontal(|ui| {
                    for (label, samples) in choices {
                        let unit = if flame { "passes" } else { syfrack_i18n::tr!("échantillons") };
                        if preset(ui, width, label, &format!("{samples} {unit}"), s.samples == samples)
                            .on_hover_text(if flame { syfrack_i18n::tr!("Accumuler davantage de points pour stabiliser les filaments et réduire le grain.") }
                                else { syfrack_i18n::tr!("Lisser les contours et les détails fins de la fractale.") }).clicked() { s.samples = samples; }
                    }
                });
            } else {
                ui.horizontal(|ui| {
                    for (label, samples, bounces) in QUALITY {
                        let label = syfrack_i18n::text(label);
                        if preset(ui, width, label, &syfrack_i18n::tr_format!("{samples} éch. · {bounces} rebonds", bounces = bounces, samples = samples), s.samples == samples && s.bounces == bounces)
                            .on_hover_text(syfrack_i18n::tr!("Les échantillons réduisent le grain ; les rebonds enrichissent la lumière indirecte."))
                            .clicked() { s.samples = samples; s.bounces = bounces; }
                    }
                });
                ui.add_space(8.0);
                row(ui, syfrack_i18n::tr!("Rebonds de lumière"), |ui| {
                    let r = ui.add(egui::DragValue::new(&mut s.bounces).range(0..=32).suffix(syfrack_i18n::tr!(" rebonds")))
                        .on_hover_text(syfrack_i18n::tr!("0 : lumière directe. Les rebonds supplémentaires propagent les reflets et la lumière indirecte, au prix d’un calcul plus long."));
                    #[cfg(test)]
                    ui.data_mut(|d| d.insert_temp(egui::Id::new("render-bounces"), r.rect));
                    let _ = r;
                });
                caption(ui, syfrack_i18n::tr!("Précision des reliefs"));
                ui.horizontal(|ui| {
                    for (label, tolerance, steps) in [("Standard", 1.0, 512), (syfrack_i18n::tr!("Fine"), 0.25, 768), (syfrack_i18n::tr!("Très fine"), 0.125, 1024)] {
                        if ui.add_sized([width, 30.0], egui::Button::new(label).selected(s.pixel_tolerance == tolerance))
                            .on_hover_text(syfrack_i18n::tr!("Une précision plus fine conserve les petits ajours et demande davantage de calcul."))
                            .clicked() { s.pixel_tolerance = tolerance; s.max_steps = steps; }
                    }
                });
            }
            ui.add_space(8.0);
            let custom = egui::CollapsingHeader::new(syfrack_i18n::tr!("Réglages personnalisés")).id_salt("render-custom").show(ui, |ui| {
                ui.add_space(4.0);
                caption(ui, syfrack_i18n::tr!("ÉCHANTILLONNAGE"));
                row(ui, if flame { "Passes" } else { syfrack_i18n::tr!("Échantillons / pixel") }, |ui| {
                    ui.add(egui::DragValue::new(&mut s.samples).range(1..=1_000_000).speed(4));
                });
                if let Some(f) = &mut s.flame {
                    row(ui, syfrack_i18n::tr!("Points / pixel / passe"), |ui| {
                        ui.add(egui::DragValue::new(&mut f.points_per_sample).range(1..=32))
                            .on_hover_text(syfrack_i18n::tr!("Densité de points calculés à chaque passe. Augmenter pour stabiliser les zones peu denses."));
                    });
                    row(ui, syfrack_i18n::tr!("Suréchantillonnage"), |ui| {
                        ui.add(egui::DragValue::new(&mut f.supersampling).range(1..=3).prefix("× "))
                            .on_hover_text(syfrack_i18n::tr!("×2 multiplie par quatre la grille d’accumulation, ×3 par neuf. Améliore les contours fins et demande davantage de mémoire."));
                    });
                    row(ui, syfrack_i18n::tr!("Rayon du filtre"), |ui| {
                        ui.add(egui::DragValue::new(&mut f.filter_radius).range(0.3..=3.0).speed(0.05).custom_formatter(|n, _| crate::ui::widgets::format_value(n as f32, 2)).custom_parser(|s| s.replace(',', ".").parse().ok()).suffix(" px"))
                            .on_hover_text(syfrack_i18n::tr!("Reconstruction orientée selon les filaments. Un rayon plus grand adoucit les zones peu denses."));
                    });
                }
                if !flat {
                    ui.add_space(8.0);
                    caption(ui, syfrack_i18n::tr!("GÉOMÉTRIE ET LUMIÈRE"));
                    row(ui, syfrack_i18n::tr!("Tolérance du tracé"), |ui| {
                        ui.add(egui::DragValue::new(&mut s.pixel_tolerance).range(0.125..=1.0).speed(0.025).max_decimals(3).custom_formatter(|n, _| crate::ui::widgets::format_value(n as f32, 3)).custom_parser(|s| s.replace(',', ".").parse().ok()).suffix(" px"))
                            .on_hover_text(syfrack_i18n::tr!("Une valeur plus faible préserve les petits ajours. 1 px : standard ; 0,25 px : fin ; 0,125 px : très fin."));
                    });
                    row(ui, syfrack_i18n::tr!("Pas de recherche"), |ui| {
                        ui.add(egui::DragValue::new(&mut s.max_steps).range(64..=4096).speed(32))
                            .on_hover_text(syfrack_i18n::tr!("Budget maximal de parcours des rayons. Augmenter pour les intérieurs profonds et les reliefs très fins."));
                    });
                    ui.checkbox(&mut s.guiding, syfrack_i18n::tr!("Guidage de la lumière indirecte · expérimental"))
                        .on_hover_text(syfrack_i18n::tr!("À comparer sur les rendus longs. L’apprentissage commence après 16 échantillons."));
                }
                ui.add_space(8.0);
                caption(ui, syfrack_i18n::tr!("RÉACTIVITÉ DU CALCUL"));
                row(ui, syfrack_i18n::tr!("Lot maximal"), |ui| {
                    ui.add(egui::DragValue::new(&mut s.samples_per_dispatch).range(1..=32))
                        .on_hover_text(syfrack_i18n::tr!("Passes ou échantillons par lot. Des lots plus petits permettent de suivre plus souvent la progression."));
                });
                if !flat {
                    row(ui, syfrack_i18n::tr!("Tuiles maximales"), |ui| {
                        egui::ComboBox::from_id_salt("render-tile-size").width(140.0).selected_text(format!("{} × {} px", s.tile_size, s.tile_size))
                            .show_ui(ui, |ui| {
                                for size in [16, 32, 64, 128, 256, 512] { ui.selectable_value(&mut s.tile_size, size, format!("{size} × {size} px")); }
                            }).response.on_hover_text(syfrack_i18n::tr!("Le moteur commence avec de petites tuiles et les agrandit jusqu’à ce plafond si le calcul est rapide."));
                    });
                }
            });
            // The capture example drives the real header through pointer input.
            let viewport = ui.ctx().viewport_id();
            ui.data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("render-custom-header"),
                    (viewport, custom.header_response.rect),
                )
            });
            if let Some(f) = s.flame {
                ui.add_space(8.0);
                let points = u64::from(s.width)
                    * u64::from(s.height)
                    * u64::from(s.samples)
                    * u64::from(f.points_per_sample);
                caption(
                    ui,
                    &syfrack_i18n::tr_format!(
                        "{:.2} milliards de points · histogramme {:.0} Mio",
                        points as f64 / 1e9,
                        s.flame_cells().unwrap() as f64 * 32.0 / 1_048_576.0
                    ),
                );
                if s.flame_limit_exceeded() {
                    ui.colored_label(theme::ERROR, syfrack_i18n::tr!("La grille dépasse 512 Mio. Réduisez la définition ou le suréchantillonnage pour lancer le rendu."));
                }
            }
        });
        ui.add_space(12.0);
        card(ui, |ui| {
            ui.label(
                RichText::new(syfrack_i18n::tr!("Fichiers produits"))
                    .size(17.0)
                    .strong(),
            );
            ui.add_space(6.0);
            ui.columns(3, |columns| {
                for (column, (title, detail)) in columns.iter_mut().zip([
                    (
                        syfrack_i18n::tr!("PNG 16 bits"),
                        syfrack_i18n::tr!("Image à partager"),
                    ),
                    (
                        syfrack_i18n::tr!("EXR linéaire"),
                        syfrack_i18n::tr!("Données brutes"),
                    ),
                    (
                        syfrack_i18n::tr!("Scène RON"),
                        syfrack_i18n::tr!("Projet réutilisable"),
                    ),
                ]) {
                    column.label(RichText::new(title).strong());
                    caption(column, detail);
                }
            });
            ui.add_space(8.0);
            if !flat {
                ui.checkbox(&mut s.denoise, syfrack_i18n::tr!("Débruiter le PNG"));
            } else {
                caption(
                    ui,
                    if s.flame.is_some() {
                        syfrack_i18n::tr!("Reconstruction adaptée aux filaments de la flamme.")
                    } else {
                        syfrack_i18n::tr!("Antialiasing des contours et des détails 2D.")
                    },
                );
            }
            caption(
                ui,
                syfrack_i18n::tr!("L’EXR conserve toujours les données brutes du rendu."),
            );
        });
        self.size_active = size_hot;
    }

    fn destination_form(&mut self, ui: &mut Ui, state: &AppState, sidebar: bool) {
        let s = &mut self.settings;
        ui.label(RichText::new("Destination").size(17.0).strong());
        ui.add_space(8.0);
        caption(ui, syfrack_i18n::tr!("Nom du rendu"));
        ui.add(
            egui::TextEdit::singleline(&mut s.name)
                .hint_text(default_name(state.path.as_deref()))
                .margin(Vec2::new(9.0, 7.0))
                .desired_width(f32::INFINITY),
        );
        ui.add_space(8.0);
        caption(ui, syfrack_i18n::tr!("Dossier de sortie"));
        let browse = |ui: &mut Ui, s: &Settings| {
            let response = ui.add_sized(
                [ui.available_width().min(130.0), 32.0],
                egui::Button::new(syfrack_i18n::tr!("Parcourir…")),
            );
            #[cfg(test)]
            ui.data_mut(|d| d.insert_temp(egui::Id::new("render-browse-button"), response.rect));
            if response.clicked() {
                file_browser::open_folder_dialog(
                    ui,
                    state,
                    Path::new(&s.dir),
                    directory_result_id(),
                );
            }
        };
        if sidebar {
            ui.add(
                egui::TextEdit::singleline(&mut s.dir)
                    .margin(Vec2::new(9.0, 7.0))
                    .desired_width(f32::INFINITY),
            )
            .on_hover_text(&s.dir);
            browse(ui, s);
        } else {
            ui.horizontal(|ui| {
                ui.add_sized(
                    [ui.available_width() - 138.0, 32.0],
                    egui::TextEdit::singleline(&mut s.dir).margin(Vec2::new(9.0, 7.0)),
                )
                .on_hover_text(&s.dir);
                browse(ui, s);
            });
        }
        ui.add_space(10.0);
        caption(
            ui,
            syfrack_i18n::tr!(
                "Les trois fichiers portent ce nom et sont enregistrés dans ce dossier."
            ),
        );
    }

    fn summary(&mut self, ui: &mut Ui, state: &AppState, busy: bool) {
        card(ui, |ui| {
            ui.label(
                RichText::new(syfrack_i18n::tr!("Votre export"))
                    .size(18.0)
                    .strong(),
            );
            ui.add_space(10.0);
            let s = &self.settings;
            let width = ui.available_width();
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 100.0), Sense::hover());
            ui.painter().rect_filled(rect, 4.0, theme::FIELD);
            let aspect = s.width as f32 / s.height.max(1) as f32;
            let w = (width - 40.0).min(72.0 * aspect);
            let frame = egui::Rect::from_center_size(rect.center(), Vec2::new(w, w / aspect));
            ui.painter().rect_filled(frame, 2.0, theme::SELECTED);
            ui.painter().rect_stroke(
                frame,
                2.0,
                Stroke::new(1.0, theme::ACCENT),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                aspect_label(s.width, s.height),
                FontId::proportional(14.0),
                theme::TEXT,
            );
            ui.add_space(8.0);
            ui.label(
                RichText::new(format!("{} × {}", s.width, s.height))
                    .size(23.0)
                    .strong(),
            );
            caption(
                ui,
                &syfrack_i18n::tr_format!(
                    "{:.2} mégapixels · {}",
                    s.width as f64 * s.height as f64 / 1_000_000.0,
                    syfrack_i18n::text(if s.width == s.height {
                        "Carré"
                    } else if s.width > s.height {
                        "Paysage"
                    } else {
                        "Portrait"
                    })
                ),
            );
            ui.add_space(8.0);
            ui.separator();
            metric(
                ui,
                if s.flame.is_some() {
                    "Passes"
                } else {
                    syfrack_i18n::tr!("Échantillons / pixel")
                },
                &s.samples.to_string(),
            );
            if s.dimension == syfrack_core::Dimension::ThreeD {
                metric(ui, syfrack_i18n::tr!("Rebonds"), &s.bounces.to_string());
                metric(
                    ui,
                    syfrack_i18n::tr!("Reliefs"),
                    if s.pixel_tolerance <= 0.125 {
                        syfrack_i18n::tr!("Très fins")
                    } else if s.pixel_tolerance <= 0.25 {
                        syfrack_i18n::tr!("Fins")
                    } else {
                        "Standard"
                    },
                );
                if s.guiding {
                    metric(
                        ui,
                        syfrack_i18n::tr!("Guidage"),
                        syfrack_i18n::tr!("Activé"),
                    );
                }
            }
            if let Some(f) = s.flame {
                metric(
                    ui,
                    syfrack_i18n::tr!("Points / pixel / passe"),
                    &f.points_per_sample.to_string(),
                );
                metric(
                    ui,
                    syfrack_i18n::tr!("Suréchantillonnage"),
                    &format!("×{}", f.supersampling),
                );
                metric(
                    ui,
                    syfrack_i18n::tr!("Filtre"),
                    &format!(
                        "{} px",
                        crate::ui::widgets::format_value(f.filter_radius, 2)
                    ),
                );
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(8.0);
            ui.add_enabled_ui(!busy, |ui| self.destination_form(ui, state, true));
        });
    }

    fn main_column(&mut self, ui: &mut Ui, busy: bool, flat: bool) {
        if busy {
            card(ui, |ui| {
                ui.label(
                    RichText::new(syfrack_i18n::tr!("Suivi du rendu"))
                        .size(18.0)
                        .strong(),
                );
                self.status_line(ui);
                self.progress_block(ui);
                if self.progress.is_none() {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        caption(ui, syfrack_i18n::tr!("Préparation de la scène…"));
                    });
                }
            });
            ui.add_space(12.0);
            egui::CollapsingHeader::new(syfrack_i18n::tr!("Réglages utilisés"))
                .id_salt("render-used-settings")
                .show(ui, |ui| {
                    ui.add_enabled_ui(false, |ui| self.settings_form(ui, flat));
                });
        } else {
            if self.status != Status::Idle {
                card(ui, |ui| {
                    self.status_line(ui);
                    self.progress_block(ui);
                });
                ui.add_space(12.0);
            }
            self.settings_form(ui, flat);
        }
    }

    fn footer(&self, ui: &mut Ui, hide: &mut bool) -> Action {
        let mut action = Action::None;
        let active = matches!(self.status, Status::Running | Status::Pausing);
        let resume = self.can_resume();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(if active {
                        syfrack_i18n::tr!("Calcul en arrière-plan")
                    } else {
                        syfrack_i18n::tr!("PNG + EXR + scène")
                    })
                    .strong(),
                );
                caption(
                    ui,
                    if active {
                        syfrack_i18n::tr!("Vous pouvez fermer cette fenêtre.")
                    } else {
                        syfrack_i18n::tr!("Tous les fichiers sont enregistrés ensemble.")
                    },
                );
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (label, enabled, next) = if self.status == Status::Pausing {
                    (syfrack_i18n::tr!("Enregistrement…"), false, Action::None)
                } else if active {
                    (syfrack_i18n::tr!("Mettre en pause"), true, Action::Pause)
                } else if resume {
                    (
                        syfrack_i18n::tr!("Reprendre le rendu"),
                        true,
                        Action::Resume,
                    )
                } else {
                    (
                        syfrack_i18n::tr!("Lancer le rendu"),
                        self.job.is_none() && !self.settings.flame_limit_exceeded(),
                        Action::Start,
                    )
                };
                let start = ui.add_enabled(
                    enabled,
                    egui::Button::new(RichText::new(label).color(theme::BG))
                        .fill(theme::ACCENT)
                        .min_size(Vec2::new(166.0, 40.0)),
                );
                #[cfg(test)]
                ui.data_mut(|d| d.insert_temp(egui::Id::new("render-start-button"), start.rect));
                if start.clicked() {
                    action = next;
                }
                if active {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(syfrack_i18n::tr!("Arrêter")).color(theme::ERROR),
                            )
                            .min_size(Vec2::new(88.0, 40.0)),
                        )
                        .clicked()
                    {
                        action = Action::Cancel;
                    }
                } else if resume {
                    if ui
                        .add(
                            egui::Button::new(syfrack_i18n::tr!("Nouveau rendu"))
                                .min_size(Vec2::new(120.0, 40.0)),
                        )
                        .on_hover_text(syfrack_i18n::tr!(
                            "Lancer un nouveau rendu avec les réglages et la destination affichés."
                        ))
                        .clicked()
                    {
                        action = Action::Start;
                    }
                } else if ui
                    .add(
                        egui::Button::new(syfrack_i18n::tr!("Fermer"))
                            .min_size(Vec2::new(88.0, 40.0)),
                    )
                    .clicked()
                {
                    *hide = true;
                }
            });
        });
        action
    }
}

fn card<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(theme::BG)
        .stroke(Stroke::new(1.0, theme::SEPARATOR))
        .corner_radius(6.0)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            add(ui)
        })
        .inner
}

fn section(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(17.0).strong());
    caption(ui, subtitle);
    ui.add_space(6.0);
}

fn caption(ui: &mut Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(theme::TEXT_DIM)).wrap());
}

fn metric(ui: &mut Ui, label: &str, value: &str) {
    ui.horizontal(|ui| {
        caption(ui, label);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(value);
        });
    });
}

fn preset(
    ui: &mut Ui,
    width: f32,
    title: &str,
    description: &str,
    selected: bool,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        title,
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(14.0),
            color: if selected { theme::ACCENT } else { theme::TEXT },
            ..Default::default()
        },
    );
    job.append(
        &format!("\n{description}"),
        0.0,
        egui::TextFormat {
            font_id: FontId::proportional(11.0),
            color: theme::TEXT_DIM,
            ..Default::default()
        },
    );
    let response = ui
        .add_sized(
            [width, 58.0],
            egui::Button::new("")
                .left_text(job)
                .selected(selected)
                .fill(if selected {
                    theme::SELECTED
                } else {
                    theme::FIELD
                })
                .stroke(Stroke::new(
                    1.0,
                    if selected {
                        theme::ACCENT
                    } else {
                        theme::SEPARATOR
                    },
                ))
                .corner_radius(4.0),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if ui.is_enabled() && (response.hovered() || response.has_focus()) {
        ui.painter().rect_stroke(
            response.rect.shrink(0.5),
            4.0,
            Stroke::new(if response.has_focus() { 2.0 } else { 1.0 }, theme::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
    response
}

fn aspect_label(width: u32, height: u32) -> String {
    let (mut a, mut b) = (width.max(1), height.max(1));
    while b != 0 {
        (a, b) = (b, a % b);
    }
    let (w, h) = (width / a, height / a);
    if w <= 32 && h <= 32 {
        format!("{w}:{h}")
    } else {
        format!("{:.2}:1", width as f32 / height.max(1) as f32)
    }
}
