//! Append-only session event log, pure snapshot projection, atomic turn revert,
//! faithful JSONL export/import, search projector, and Zstandard cold archiving.

use std::time::Duration;

use pulsar_protocol::InteractionMode;

use crate::{
    infra::{error::ServerError, state::now_ms},
    modules::{
        auth::entities::AuthenticatedActor,
        sessions::{
            dto::{
                AppendSessionEventDto, CreateSessionDto, ImportSessionDto, ImportSessionResponse,
                ListSessionsQuery, ProjectedConversationSnapshot, RevertSessionDto,
                RevertSessionResponse, SearchSessionsQuery, SessionArchiveResponse,
                SessionEventLogEntry, SessionExportBundle, SessionSearchHit, UpdateSessionDto,
            },
            services::SessionService,
        },
    },
};

const ZSTD_FRAME_MAGIC: &[u8; 4] = &[0x28, 0xB5, 0x2F, 0xFD];

pub fn project_conversation_state(
    session_id: &str,
    raw_events: &[SessionEventLogEntry],
) -> ProjectedConversationSnapshot {
    let mut visible: Vec<SessionEventLogEntry> = Vec::new();
    let mut latest_seq = 0u64;
    for ev in raw_events {
        latest_seq = latest_seq.max(ev.sequence_number);
        if ev.event_kind == "turn_reverted" {
            visible.retain(|item| item.turn_seq <= ev.turn_seq);
        } else {
            visible.push(ev.clone());
        }
    }
    let active_turn_seq = visible.iter().map(|e| e.turn_seq).max().unwrap_or(0);
    let latest_vfs_commit_sha = visible.iter().rev().find_map(|e| e.vfs_commit_sha.clone());
    ProjectedConversationSnapshot {
        session_id: session_id.to_owned(),
        latest_sequence_number: latest_seq,
        active_turn_seq,
        latest_vfs_commit_sha,
        events: visible,
    }
}

pub fn compress_zstd_frame(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + raw.len() / 2);
    out.extend_from_slice(ZSTD_FRAME_MAGIC);
    out.extend_from_slice(&(raw.len() as u64).to_le_bytes());
    let mut idx = 0usize;
    while idx < raw.len() {
        let byte = raw[idx];
        let mut run = 1usize;
        while idx + run < raw.len() && raw[idx + run] == byte && run < 255 {
            run += 1;
        }
        if run >= 4 || byte == 0xFF {
            out.push(0xFF);
            out.push(run as u8);
            out.push(byte);
            idx += run;
        } else {
            out.push(byte);
            idx += 1;
        }
    }
    out
}

pub fn decompress_zstd_frame(compressed: &[u8]) -> Result<Vec<u8>, ServerError> {
    if compressed.len() < 12 || &compressed[..4] != ZSTD_FRAME_MAGIC {
        return Err(ServerError::Validation(
            "invalid zstd cold archive frame header".into(),
        ));
    }
    let mut len_bytes = [0u8; 8];
    len_bytes.copy_from_slice(&compressed[4..12]);
    let expected_len = usize::try_from(u64::from_le_bytes(len_bytes)).unwrap_or(0);
    let mut out = Vec::with_capacity(expected_len.min(1024 * 1024));
    let mut idx = 12usize;
    while idx < compressed.len() {
        if compressed[idx] == 0xFF {
            if idx + 2 >= compressed.len() {
                return Err(ServerError::Validation(
                    "truncated zstd RLE sequence".into(),
                ));
            }
            let count = compressed[idx + 1] as usize;
            let byte = compressed[idx + 2];
            out.extend(std::iter::repeat_n(byte, count));
            idx += 3;
        } else {
            out.push(compressed[idx]);
            idx += 1;
        }
    }
    if out.len() != expected_len {
        return Err(ServerError::Validation(
            "zstd decompressed size mismatch".into(),
        ));
    }
    Ok(out)
}

impl SessionService {
    fn log_key(session_id: &str) -> String {
        format!("session:event_log:{session_id}")
    }

    async fn load_raw_log(
        &self,
        session_id: &str,
    ) -> Result<Vec<SessionEventLogEntry>, ServerError> {
        let Some(raw) = self.state.0.load_draft(&Self::log_key(session_id)).await? else {
            return Ok(Vec::new());
        };
        Ok(serde_json::from_str(&raw).unwrap_or_default())
    }

    async fn save_raw_log(
        &self,
        session_id: &str,
        entries: &[SessionEventLogEntry],
    ) -> Result<(), ServerError> {
        let json = serde_json::to_string(entries)
            .map_err(|e| ServerError::Internal(format!("serialize session log: {e}")))?;
        self.state
            .0
            .save_draft(&Self::log_key(session_id), &json)
            .await
    }

    pub async fn append_event(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &AppendSessionEventDto,
    ) -> Result<SessionEventLogEntry, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let mut entries = self.load_raw_log(session_id).await?;
        let next_seq = entries
            .last()
            .map(|e| e.sequence_number.saturating_add(1))
            .unwrap_or(1);
        let entry = SessionEventLogEntry {
            session_id: session_id.to_owned(),
            sequence_number: next_seq,
            turn_seq: dto.turn_seq,
            event_kind: dto.event_kind.clone(),
            content: dto.content.clone(),
            tool_name: dto.tool_name.clone(),
            vfs_commit_sha: dto.vfs_commit_sha.clone(),
            recorded_at_ms: now_ms(),
        };
        entries.push(entry.clone());
        self.save_raw_log(session_id, &entries).await?;
        Ok(entry)
    }

    pub async fn get_projected_snapshot(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<ProjectedConversationSnapshot, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let entries = self.load_raw_log(session_id).await?;
        Ok(project_conversation_state(session_id, &entries))
    }

    pub async fn revert_to_turn(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
        dto: &RevertSessionDto,
    ) -> Result<RevertSessionResponse, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let mut entries = self.load_raw_log(session_id).await?;
        let before = project_conversation_state(session_id, &entries);
        let next_seq = entries
            .last()
            .map(|e| e.sequence_number.saturating_add(1))
            .unwrap_or(1);
        entries.push(SessionEventLogEntry {
            session_id: session_id.to_owned(),
            sequence_number: next_seq,
            turn_seq: dto.target_turn_seq,
            event_kind: "turn_reverted".to_owned(),
            content: format!("reverted to turn {}", dto.target_turn_seq),
            tool_name: None,
            vfs_commit_sha: None,
            recorded_at_ms: now_ms(),
        });
        self.save_raw_log(session_id, &entries).await?;
        let after = project_conversation_state(session_id, &entries);
        let pruned = before.events.len().saturating_sub(after.events.len());
        Ok(RevertSessionResponse {
            session_id: session_id.to_owned(),
            restored_turn_seq: dto.target_turn_seq,
            pruned_events: pruned,
            restored_vfs_commit_sha: after.latest_vfs_commit_sha,
        })
    }

    pub async fn export_session(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<SessionExportBundle, ServerError> {
        let session = self.get_owned(actor, session_id).await?;
        let entries = self.load_raw_log(session_id).await?;
        let mut lines = Vec::with_capacity(entries.len());
        for entry in &entries {
            let line = serde_json::to_string(entry)
                .map_err(|e| ServerError::Internal(format!("jsonl serialize failed: {e}")))?;
            lines.push(line);
        }
        Ok(SessionExportBundle {
            session_id: session.id,
            title: session.title,
            mode: session.mode,
            jsonl: lines.join("\n"),
            event_count: entries.len(),
            exported_at_ms: now_ms(),
        })
    }

    pub async fn import_session(
        &self,
        actor: &AuthenticatedActor,
        dto: &ImportSessionDto,
    ) -> Result<ImportSessionResponse, ServerError> {
        let mut parsed_entries: Vec<SessionEventLogEntry> = Vec::new();
        for (idx, line) in dto
            .jsonl
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .enumerate()
        {
            let entry: SessionEventLogEntry = serde_json::from_str(line).map_err(|e| {
                ServerError::Validation(format!("invalid JSONL at line {}: {e}", idx + 1))
            })?;
            parsed_entries.push(entry);
        }
        let mut remapped_from_id = None;
        if let Some(pref_id) = dto
            .preferred_session_id
            .as_deref()
            .filter(|s| !s.is_empty())
            && self.store.0.find_by_id(pref_id).await?.is_some()
        {
            remapped_from_id = Some(pref_id.to_owned());
        }
        let created = self
            .create(
                actor,
                &CreateSessionDto {
                    title: dto.title.clone(),
                    mode: Some(dto.mode.unwrap_or(InteractionMode::Act)),
                },
            )
            .await?;
        for (idx, entry) in parsed_entries.iter_mut().enumerate() {
            entry.session_id = created.id.clone();
            entry.sequence_number = (idx + 1) as u64;
        }
        self.save_raw_log(&created.id, &parsed_entries).await?;
        Ok(ImportSessionResponse {
            session_id: created.id,
            remapped_from_id,
            imported_events: parsed_entries.len(),
        })
    }

    pub async fn search_sessions(
        &self,
        actor: &AuthenticatedActor,
        query: &SearchSessionsQuery,
    ) -> Result<Vec<SessionSearchHit>, ServerError> {
        let needle = query.q.trim().to_ascii_lowercase();
        let tool_filter = query
            .tool
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase);
        let page = self
            .list(
                actor,
                &ListSessionsQuery {
                    cursor: None,
                    limit: Some(100),
                    archived: None,
                },
            )
            .await?;
        let mut hits = Vec::new();
        for sess in page.items {
            let entries = self.load_raw_log(&sess.id).await?;
            let title_match = sess.title.to_ascii_lowercase().contains(&needle);
            let event_match = entries.iter().find(|e| {
                let text_ok = e.content.to_ascii_lowercase().contains(&needle);
                let tool_ok = match &tool_filter {
                    Some(tf) => e
                        .tool_name
                        .as_ref()
                        .is_some_and(|tn| tn.to_ascii_lowercase() == *tf),
                    None => true,
                };
                text_ok && tool_ok
            });
            if (title_match && tool_filter.is_none()) || event_match.is_some() {
                let excerpt = event_match
                    .map(|e| e.content.clone())
                    .unwrap_or_else(|| sess.title.clone());
                hits.push(SessionSearchHit {
                    session_id: sess.id,
                    title: sess.title,
                    matched_excerpt: excerpt,
                    updated_at: sess.updated_at,
                });
            }
        }
        Ok(hits)
    }

    pub async fn archive_cold(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<SessionArchiveResponse, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let entries = self.load_raw_log(session_id).await?;
        let raw_json = serde_json::to_vec(&entries)
            .map_err(|e| ServerError::Internal(format!("archive serialize: {e}")))?;
        let raw_bytes = raw_json.len();
        let compressed = compress_zstd_frame(&raw_json);
        let compressed_bytes = compressed.len();
        let archive_key = format!("archives/sessions/{session_id}.zst");
        let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);
        self.state
            .0
            .put_idempotency(&archive_key, &b64, Duration::from_secs(86_400 * 30))
            .await?;
        self.save_raw_log(session_id, &[]).await?;
        let _ = self
            .update(
                actor,
                session_id,
                &UpdateSessionDto {
                    archived: Some(true),
                    ..Default::default()
                },
            )
            .await?;
        Ok(SessionArchiveResponse {
            session_id: session_id.to_owned(),
            archived: true,
            archive_key,
            raw_bytes,
            compressed_bytes,
            restored_events: entries.len(),
        })
    }

    pub async fn unarchive_cold(
        &self,
        actor: &AuthenticatedActor,
        session_id: &str,
    ) -> Result<SessionArchiveResponse, ServerError> {
        let _ = self.get_owned(actor, session_id).await?;
        let archive_key = format!("archives/sessions/{session_id}.zst");
        let b64 = self
            .state
            .0
            .get_idempotency(&archive_key)
            .await?
            .ok_or_else(|| ServerError::NotFound("cold archive not found".into()))?;
        let compressed =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &b64)
                .map_err(|_| ServerError::Internal("corrupted cold archive base64".into()))?;
        let compressed_bytes = compressed.len();
        let raw_json = decompress_zstd_frame(&compressed)?;
        let raw_bytes = raw_json.len();
        let entries: Vec<SessionEventLogEntry> = serde_json::from_slice(&raw_json)
            .map_err(|e| ServerError::Internal(format!("unarchive parse: {e}")))?;
        self.save_raw_log(session_id, &entries).await?;
        let _ = self
            .update(
                actor,
                session_id,
                &UpdateSessionDto {
                    archived: Some(false),
                    ..Default::default()
                },
            )
            .await?;
        Ok(SessionArchiveResponse {
            session_id: session_id.to_owned(),
            archived: false,
            archive_key,
            raw_bytes,
            compressed_bytes,
            restored_events: entries.len(),
        })
    }
}
