use crate::{on_core, publish, require_main, DesktopState};
use nero_core::{
    agent_control::meeting_edit_version, CoreHandle, Meeting, MeetingAttendee, PeopleCatalog,
    Person, PersonGroup, PromotedAttendee,
};
use serde::Serialize;
use tauri::{AppHandle, State, WebviewWindow};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingAttendanceView {
    pub meeting: Meeting,
    pub edit_version: String,
}

impl MeetingAttendanceView {
    fn new(meeting: Meeting) -> nero_core::Result<Self> {
        let edit_version = meeting_edit_version(&meeting)?;
        Ok(Self {
            meeting,
            edit_version,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromotedAttendanceView {
    pub meeting: Meeting,
    pub edit_version: String,
    pub catalog: PeopleCatalog,
    pub person: Person,
}

impl PromotedAttendanceView {
    fn new(result: PromotedAttendee) -> nero_core::Result<Self> {
        let edit_version = meeting_edit_version(&result.meeting)?;
        Ok(Self {
            meeting: result.meeting,
            edit_version,
            catalog: result.catalog,
            person: result.person,
        })
    }
}

async fn mutate<T: Send + 'static>(
    app: AppHandle,
    core: CoreHandle,
    action: impl FnOnce(&CoreHandle) -> nero_core::Result<T> + Send + 'static,
) -> Result<T, String> {
    let (result, snapshot) = on_core(core, move |core| {
        let result = action(&core);
        let snapshot = core.snapshot();
        Ok((result, snapshot))
    })
    .await?;
    if let Ok(snapshot) = snapshot {
        publish(&app, &snapshot);
    }
    result.map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn people_catalog(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
) -> Result<PeopleCatalog, String> {
    require_main(&window)?;
    on_core(state.core.clone(), |core| core.people_catalog()).await
}

#[tauri::command]
pub async fn replace_people_catalog(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    expected_version: String,
    people: Vec<Person>,
    groups: Vec<PersonGroup>,
) -> Result<PeopleCatalog, String> {
    require_main(&window)?;
    mutate(app, state.core.clone(), move |core| {
        core.replace_people_catalog(&expected_version, people, groups)
    })
    .await
}

#[tauri::command]
pub async fn meeting_attendees(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    id: String,
) -> Result<MeetingAttendanceView, String> {
    require_main(&window)?;
    on_core(state.core.clone(), move |core| {
        let meeting = core
            .list_meetings()?
            .into_iter()
            .find(|meeting| meeting.id == id && meeting.deleted_at.is_none())
            .ok_or(nero_core::CoreError::MeetingNotFound)?;
        MeetingAttendanceView::new(meeting)
    })
    .await
}

#[tauri::command]
pub async fn replace_meeting_attendees(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    id: String,
    expected_version: String,
    attendees: Vec<MeetingAttendee>,
) -> Result<MeetingAttendanceView, String> {
    require_main(&window)?;
    mutate(app, state.core.clone(), move |core| {
        MeetingAttendanceView::new(core.replace_meeting_attendees(
            &id,
            &expected_version,
            attendees,
        )?)
    })
    .await
}

#[tauri::command]
pub async fn promote_meeting_attendee(
    window: WebviewWindow,
    app: AppHandle,
    state: State<'_, DesktopState>,
    id: String,
    expected_version: String,
    expected_catalog_version: String,
    attendee_id: String,
    name: String,
) -> Result<PromotedAttendanceView, String> {
    require_main(&window)?;
    mutate(app, state.core.clone(), move |core| {
        PromotedAttendanceView::new(core.promote_meeting_attendee(
            &id,
            &expected_version,
            &expected_catalog_version,
            &attendee_id,
            name,
        )?)
    })
    .await
}
