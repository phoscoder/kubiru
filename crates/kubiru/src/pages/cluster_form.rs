//! Adding and editing a cluster.

use gpui_kit::assets::IconName;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, Focusable as _, FontWeight, IntoElement,
    ParentElement, Render, SharedString, Styled, Window, div,
};
use kubiru_core::cluster::{Field, TokenRequirement, ValidationErrors};
use kubiru_core::kube::KubeClient;
use kubiru_core::{
    Cluster, ClusterDraft, ClusterId, Credentials, RegistryError, Secret, TlsVerification,
};

use crate::session::Session;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormEvent {
    Saved(ClusterId),
    Cancelled,
}

#[derive(Clone, Debug)]
enum Status {
    Idle,
    Testing,
    Saving,
    Tested(Result<String, String>),
    Failed(String),
}

pub struct ClusterForm {
    session: Entity<Session>,
    /// `None` when adding.
    editing: Option<Cluster>,
    name: Entity<InputState>,
    api_url: Entity<InputState>,
    token: Entity<InputState>,
    ca: Entity<TextareaState>,
    description: Entity<InputState>,
    insecure: bool,
    errors: ValidationErrors,
    status: Status,
}

impl EventEmitter<FormEvent> for ClusterForm {}

impl ClusterForm {
    pub fn new(
        session: Entity<Session>,
        editing: Option<Cluster>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let value = |f: fn(&Cluster) -> String| editing.as_ref().map(f).unwrap_or_default();
        let input = |placeholder: &'static str,
                     value: String,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            })
        };
        let ca_value = match editing.as_ref().map(|c| &c.tls) {
            Some(TlsVerification::CustomCa(pem)) => pem.clone(),
            _ => String::new(),
        };
        let token_placeholder = if editing.is_some() {
            "Leave blank to keep the stored token"
        } else {
            "eyJhbGciOiJSUzI1NiIsImtpZCI6\u{2026}"
        };
        Self {
            name: input("Production", value(|c| c.name.clone()), window, cx),
            api_url: input(
                "https://10.0.0.1:6443",
                value(|c| c.api_url.to_string()),
                window,
                cx,
            ),
            token: cx.new(|cx| {
                InputState::new(window, cx)
                    .masked(true)
                    .placeholder(token_placeholder)
            }),
            ca: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(4)
                    .placeholder("-----BEGIN CERTIFICATE-----  (optional)")
                    .default_value(ca_value)
            }),
            description: input(
                "What this cluster is for (optional)",
                value(|c| c.description.clone()),
                window,
                cx,
            ),
            insecure: editing.as_ref().is_some_and(|c| c.tls.is_insecure()),
            editing,
            session,
            errors: ValidationErrors::default(),
            status: Status::Idle,
        }
    }

    pub fn title(&self) -> &'static str {
        if self.editing.is_some() {
            "Edit cluster"
        } else {
            "Add cluster"
        }
    }

    /// The field to focus when the form opens.
    pub fn first_field(&self, cx: &App) -> gpui_kit::FocusHandle {
        self.name.read(cx).focus_handle(cx)
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Testing | Status::Saving)
    }

    fn draft(&self, cx: &App) -> ClusterDraft {
        ClusterDraft {
            name: self.name.read(cx).value().to_string(),
            api_url: self.api_url.read(cx).value().to_string(),
            description: self.description.read(cx).value().to_string(),
            token: Secret::new(self.token.read(cx).value().to_string()),
            ca_certificate: self.ca.read(cx).value().to_string(),
            insecure_skip_tls_verify: self.insecure,
        }
    }

    fn requirement(&self) -> TokenRequirement {
        if self.editing.is_some() {
            TokenRequirement::KeepIfBlank
        } else {
            TokenRequirement::Required
        }
    }

    /// Check the address and token without saving anything.
    fn test(&mut self, cx: &mut Context<Self>) {
        let valid = match self.draft(cx).validate(self.requirement()) {
            Ok(valid) => valid,
            Err(errors) => {
                self.errors = errors;
                cx.notify();
                return;
            }
        };
        self.errors = ValidationErrors::default();
        self.status = Status::Testing;
        cx.notify();
        let registry = self.session.read(cx).registry().clone();
        let existing = self.editing.as_ref().map(|c| c.id);
        let task = cx.background_spawn(async move {
            let token = match (valid.token.clone(), existing) {
                (Some(token), _) => token,
                (None, Some(id)) => registry.credentials(id).map_err(|e| e.to_string())?.token,
                (None, None) => return Err("Paste a bearer token first.".to_owned()),
            };
            let credentials = Credentials {
                cluster: Cluster {
                    id: existing.unwrap_or(ClusterId(0)),
                    name: valid.name,
                    api_url: valid.api_url,
                    description: valid.description,
                    tls: valid.tls,
                    created_at: String::new(),
                    updated_at: String::new(),
                },
                token,
            };
            let version = KubeClient::new(&credentials)
                .and_then(|client| client.verify())
                .map_err(|e| e.to_string())?;
            Ok(format!("Connected to Kubernetes {}.", version.git_version))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.status = Status::Tested(result);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let draft = self.draft(cx);
        // Validate here first so field errors appear without a round trip.
        if let Err(errors) = draft.validate(self.requirement()) {
            self.errors = errors;
            cx.notify();
            return;
        }
        self.errors = ValidationErrors::default();
        self.status = Status::Saving;
        cx.notify();
        let registry = self.session.read(cx).registry().clone();
        let existing = self.editing.as_ref().map(|c| c.id);
        let task = cx.background_spawn(async move {
            match existing {
                Some(id) => registry.update(id, &draft),
                None => registry.create(&draft),
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(cluster) => {
                        this.status = Status::Idle;
                        cx.emit(FormEvent::Saved(cluster.id));
                    }
                    Err(RegistryError::Invalid(errors)) => {
                        this.errors = errors;
                        this.status = Status::Idle;
                    }
                    Err(e) => this.status = Status::Failed(e.to_string()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn field(
        &self,
        label: &'static str,
        field: Field,
        hint: Option<&'static str>,
        control: impl IntoElement,
        cx: &App,
    ) -> impl IntoElement {
        let error = self
            .errors
            .get(field)
            .map(|e| SharedString::from(e.to_owned()));
        v_flex()
            .gap_1p5()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
            .child(control)
            .children(match (error, hint) {
                (Some(error), _) => {
                    Some(div().text_xs().text_color(cx.theme().danger).child(error))
                }
                (None, Some(hint)) => Some(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(hint),
                ),
                (None, None) => None,
            })
    }
}

impl Render for ClusterForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let busy = self.busy();
        let status = match &self.status {
            Status::Tested(Ok(message)) => {
                Some(Alert::success("form-status", message.clone()).small())
            }
            Status::Tested(Err(message)) => Some(
                Alert::error("form-status", message.clone())
                    .title("Connection failed")
                    .small(),
            ),
            Status::Failed(message) => Some(
                Alert::error("form-status", message.clone())
                    .title("Not saved")
                    .small(),
            ),
            _ => None,
        };

        v_flex()
            .gap_4()
            .child(self.field("Name", Field::Name, None, Input::new(&self.name).disabled(busy), cx))
            .child(self.field(
                "API server",
                Field::ApiUrl,
                Some("Find it with: kubectl config view --minify -o jsonpath='{.clusters[0].cluster.server}'"),
                Input::new(&self.api_url).disabled(busy),
                cx,
            ))
            .child(self.field(
                "Bearer token",
                Field::Token,
                Some("Stored in your system keychain, never on disk. Create one with: kubectl create token <service-account>"),
                Input::new(&self.token).mask_toggle().disabled(busy),
                cx,
            ))
            .child(self.field(
                "Certificate authority",
                Field::CaCertificate,
                Some("Needed when the cluster uses a private CA. Leave empty to trust the system's CAs."),
                Textarea::new(&self.ca).disabled(busy || self.insecure),
                cx,
            ))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        Checkbox::new("insecure")
                            .label("Skip TLS certificate verification")
                            .checked(self.insecure)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.insecure = *checked;
                                cx.notify();
                            })),
                    )
                    .children(self.insecure.then(|| {
                        Alert::warning(
                            "insecure-warning",
                            "Anyone on the network path could impersonate the cluster and capture your token. Use only for local test clusters.",
                        )
                        .small()
                    })),
            )
            .child(self.field("Description", Field::Description, None, Input::new(&self.description).disabled(busy), cx))
            .children(status)
            .child(
                h_flex()
                    .justify_between()
                    .gap_2()
                    .pt_2()
                    .child(
                        Button::new("test-connection")
                            .outline()
                            .icon(IconName::Plug)
                            .label("Test connection")
                            .loading(matches!(self.status, Status::Testing))
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| this.test(cx))),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("cancel")
                                    .ghost()
                                    .label("Cancel")
                                    .disabled(matches!(self.status, Status::Saving))
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(FormEvent::Cancelled))),
                            )
                            .child(
                                Button::new("save")
                                    .primary()
                                    .label(if self.editing.is_some() { "Save changes" } else { "Add cluster" })
                                    .loading(matches!(self.status, Status::Saving))
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use gpui_kit::{TestAppContext, VisualTestContext};
    use kubiru_core::Registry;
    use kubiru_core::store::Store;
    use kubiru_core::vault::MemoryVault;

    use super::*;

    struct Fixture {
        form: Entity<ClusterForm>,
        registry: Arc<Registry>,
        events: Rc<RefCell<Vec<FormEvent>>>,
    }

    fn open(
        cx: &mut TestAppContext,
        editing: Option<Cluster>,
        registry: Option<Arc<Registry>>,
    ) -> (Fixture, &mut VisualTestContext) {
        cx.update(gpui_kit::init);
        let registry = registry.unwrap_or_else(|| {
            Arc::new(Registry::new(
                Store::open_in_memory().unwrap(),
                Arc::new(MemoryVault::default()),
            ))
        });
        let session_registry = registry.clone();
        let (form, cx) = cx.add_window_view(move |window, cx| {
            let session = cx.new(|cx| Session::new(session_registry, cx));
            ClusterForm::new(session, editing, window, cx)
        });
        let events = Rc::new(RefCell::new(Vec::new()));
        let sink = events.clone();
        cx.update(|_, cx| {
            cx.subscribe(&form, move |_, event: &FormEvent, _| {
                sink.borrow_mut().push(event.clone())
            })
            .detach();
        });
        (
            Fixture {
                form,
                registry,
                events,
            },
            cx,
        )
    }

    fn type_into(
        fixture: &Fixture,
        cx: &mut VisualTestContext,
        pick: fn(&ClusterForm) -> &Entity<InputState>,
        text: &str,
    ) {
        let text = text.to_owned();
        cx.update(|window, cx| {
            let input = pick(fixture.form.read(cx)).clone();
            input.update(cx, |i, cx| i.set_value(text, window, cx));
        });
    }

    #[gpui_kit::test]
    fn saving_an_empty_form_shows_field_errors(cx: &mut TestAppContext) {
        let (fixture, cx) = open(cx, None, None);
        cx.update(|_, cx| fixture.form.update(cx, |f, cx| f.save(cx)));
        cx.run_until_parked();
        cx.update(|_, cx| {
            let form = fixture.form.read(cx);
            assert!(form.errors.get(Field::Name).is_some());
            assert!(form.errors.get(Field::ApiUrl).is_some());
            assert!(form.errors.get(Field::Token).is_some());
        });
        assert!(fixture.events.borrow().is_empty());
        assert!(fixture.registry.clusters().unwrap().is_empty());
    }

    #[gpui_kit::test]
    fn a_valid_form_saves_the_cluster_and_its_token(cx: &mut TestAppContext) {
        let (fixture, cx) = open(cx, None, None);
        type_into(&fixture, cx, |f| &f.name, "Lab");
        type_into(&fixture, cx, |f| &f.api_url, "https://lab.example.com:6443");
        type_into(&fixture, cx, |f| &f.token, "abc.def");
        cx.update(|_, cx| fixture.form.update(cx, |f, cx| f.save(cx)));
        cx.run_until_parked();
        let clusters = fixture.registry.clusters().unwrap();
        assert_eq!(clusters.len(), 1);
        assert_eq!(*fixture.events.borrow(), [FormEvent::Saved(clusters[0].id)]);
        assert_eq!(
            fixture
                .registry
                .credentials(clusters[0].id)
                .unwrap()
                .token
                .expose(),
            "abc.def"
        );
    }

    #[gpui_kit::test]
    fn editing_prefills_fields_and_keeps_a_blank_token(cx: &mut TestAppContext) {
        let registry = Arc::new(Registry::new(
            Store::open_in_memory().unwrap(),
            Arc::new(MemoryVault::default()),
        ));
        let cluster = registry
            .create(&ClusterDraft {
                name: "Lab".into(),
                api_url: "https://lab.example.com".into(),
                token: "original".into(),
                insecure_skip_tls_verify: true,
                ..Default::default()
            })
            .unwrap();
        let (fixture, cx) = open(cx, Some(cluster.clone()), Some(registry));
        cx.update(|_, cx| {
            let form = fixture.form.read(cx);
            assert_eq!(form.name.read(cx).value().as_ref(), "Lab");
            assert!(form.insecure);
            assert_eq!(form.title(), "Edit cluster");
        });
        type_into(&fixture, cx, |f| &f.name, "Lab 2");
        cx.update(|_, cx| fixture.form.update(cx, |f, cx| f.save(cx)));
        cx.run_until_parked();
        let credentials = fixture.registry.credentials(cluster.id).unwrap();
        assert_eq!(credentials.cluster.name, "Lab 2");
        assert_eq!(credentials.token.expose(), "original");
    }

    #[gpui_kit::test]
    fn cancel_emits_cancelled(cx: &mut TestAppContext) {
        let (fixture, cx) = open(cx, None, None);
        cx.update(|_, cx| {
            fixture
                .form
                .update(cx, |_, cx| cx.emit(FormEvent::Cancelled))
        });
        assert_eq!(*fixture.events.borrow(), [FormEvent::Cancelled]);
    }
}
