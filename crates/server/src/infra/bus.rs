//! NATS JetStream control plane (`BOXER_TASKS`, `BOXER_STREAM`, `SWARM_EVENTS`)
//! and in-memory message bus for deterministic tests.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};

use agents_protocol::{
    CodecId, EventEnvelope, EventFeedLane, FencingToken, SteeringMessage, StreamPayload,
    SwarmMessage, TaskRequest,
};
use async_nats::jetstream::{self, stream};
use futures_util::StreamExt;
use tokio::sync::{Mutex, broadcast};

use crate::infra::{error::ServerError, state::BoxFuture};

pub const STREAM_BOXER_TASKS: &str = "BOXER_TASKS";
pub const STREAM_BOXER_STREAM: &str = "BOXER_STREAM";
pub const STREAM_SWARM_EVENTS: &str = "SWARM_EVENTS";
pub const RENDER_LANE_CAPACITY: usize = 4096;

#[derive(Debug, Clone)]
pub struct TwoLaneEventFeedBuffer {
    capacity: usize,
    control_lane: VecDeque<EventEnvelope>,
    render_lane: VecDeque<EventEnvelope>,
    dropped_render_events: u64,
}

impl Default for TwoLaneEventFeedBuffer {
    fn default() -> Self {
        Self::new(RENDER_LANE_CAPACITY)
    }
}

impl TwoLaneEventFeedBuffer {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            control_lane: VecDeque::new(),
            render_lane: VecDeque::new(),
            dropped_render_events: 0,
        }
    }

    pub fn push(&mut self, envelope: EventEnvelope) -> Option<EventEnvelope> {
        match envelope.payload.lane() {
            EventFeedLane::Control => {
                self.control_lane.push_back(envelope);
                None
            }
            EventFeedLane::Render => {
                let mut lagged_event = None;
                if self.render_lane.len() >= self.capacity {
                    let drop_count = (self.capacity / 2).max(1);
                    let mut dropped = 0u64;
                    for _ in 0..drop_count {
                        if self.render_lane.pop_front().is_some() {
                            dropped += 1;
                        }
                    }
                    self.dropped_render_events += dropped;
                    let lag_env = EventEnvelope {
                        seq: envelope.seq,
                        emitted_at_ms: envelope.emitted_at_ms,
                        codec: CodecId::EventsV1,
                        session_id: envelope.session_id.clone(),
                        task_id: envelope.task_id.clone(),
                        payload: StreamPayload::StreamLagged {
                            dropped_render_events: dropped,
                            buffer_capacity: self.capacity,
                        },
                    };
                    self.control_lane.push_back(lag_env.clone());
                    lagged_event = Some(lag_env);
                }
                self.render_lane.push_back(envelope);
                lagged_event
            }
        }
    }

    pub fn pop_next(&mut self) -> Option<EventEnvelope> {
        self.control_lane
            .pop_front()
            .or_else(|| self.render_lane.pop_front())
    }

    #[must_use]
    pub fn control_len(&self) -> usize {
        self.control_lane.len()
    }

    #[must_use]
    pub fn render_len(&self) -> usize {
        self.render_lane.len()
    }

    #[must_use]
    pub fn total_dropped_render_events(&self) -> u64 {
        self.dropped_render_events
    }
}

/// Control-plane message bus linking Server, Boxer workers, and Swarm agents.
pub trait MessageBus: Send + Sync + 'static {
    fn publish_task_request<'a>(
        &'a self,
        req: &'a TaskRequest,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn publish_task_cancel<'a>(
        &'a self,
        task_id: &'a str,
        session_id: &'a str,
        fencing_token: FencingToken,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn publish_steering<'a>(
        &'a self,
        msg: &'a SteeringMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn publish_swarm_message<'a>(
        &'a self,
        msg: &'a SwarmMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>>;

    fn subscribe_session_stream<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, broadcast::Receiver<EventEnvelope>>;

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>>;
}

/// NATS JetStream implementation provisioning durable streams for tasks, streams, and swarm handoffs.
#[derive(Clone)]
pub struct NatsMessageBus {
    client: async_nats::Client,
    jetstream: jetstream::Context,
    channels: Arc<Mutex<HashMap<String, broadcast::Sender<EventEnvelope>>>>,
}

impl NatsMessageBus {
    /// Connects to NATS and ensures the canonical JetStream streams exist.
    ///
    /// # Errors
    ///
    /// Returns [`ServerError::Internal`] when NATS connection or stream setup fails.
    pub async fn connect(url: &str) -> Result<Arc<Self>, ServerError> {
        let client = async_nats::connect(url)
            .await
            .map_err(|e| ServerError::Internal(format!("nats connect failed: {e}")))?;
        let js = jetstream::new(client.clone());

        ensure_stream(&js, STREAM_BOXER_TASKS, vec!["boxer.tasks.>".into()]).await?;
        ensure_stream(&js, STREAM_BOXER_STREAM, vec!["boxer.stream.>".into()]).await?;
        ensure_stream(&js, STREAM_SWARM_EVENTS, vec!["swarm.>".into()]).await?;

        let bus = Arc::new(Self {
            client: client.clone(),
            jetstream: js,
            channels: Arc::new(Mutex::new(HashMap::new())),
        });

        let mut stream_sub = client
            .subscribe("boxer.stream.>")
            .await
            .map_err(|e| ServerError::Internal(format!("nats stream subscribe failed: {e}")))?;
        let fanout_bus = Arc::clone(&bus);
        tokio::spawn(async move {
            while let Some(msg) = stream_sub.next().await {
                if let Ok(envelope) = serde_json::from_slice::<EventEnvelope>(&msg.payload) {
                    fanout_bus.fanout_local(&envelope).await;
                }
            }
        });

        Ok(bus)
    }

    async fn publish_json<T: serde::Serialize>(
        &self,
        subject: String,
        payload: &T,
    ) -> Result<(), ServerError> {
        let bytes = serde_json::to_vec(payload)
            .map_err(|e| ServerError::Internal(format!("bus serialize error: {e}")))?;
        self.jetstream
            .publish(subject, bytes.into())
            .await
            .map_err(|e| ServerError::Internal(format!("nats publish error: {e}")))?
            .await
            .map_err(|e| ServerError::Internal(format!("nats ack error: {e}")))?;
        Ok(())
    }

    async fn fanout_local(&self, envelope: &EventEnvelope) {
        let mut map = self.channels.lock().await;
        let tx = map
            .entry(envelope.session_id.clone())
            .or_insert_with(|| broadcast::channel(RENDER_LANE_CAPACITY).0);
        let _ = tx.send(envelope.clone());
    }
}

async fn ensure_stream(
    js: &jetstream::Context,
    name: &str,
    subjects: Vec<String>,
) -> Result<(), ServerError> {
    js.get_or_create_stream(stream::Config {
        name: name.to_owned(),
        subjects,
        ..Default::default()
    })
    .await
    .map_err(|e| ServerError::Internal(format!("jetstream `{name}` setup failed: {e}")))?;
    Ok(())
}

impl MessageBus for NatsMessageBus {
    fn publish_task_request<'a>(
        &'a self,
        req: &'a TaskRequest,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.publish_json("boxer.tasks.request".to_owned(), req)
                .await
        })
    }

    fn publish_task_cancel<'a>(
        &'a self,
        task_id: &'a str,
        session_id: &'a str,
        fencing_token: FencingToken,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let payload = serde_json::json!({
                "taskId": task_id,
                "sessionId": session_id,
                "fencingToken": fencing_token.0,
            });
            self.publish_json("boxer.tasks.cancel".to_owned(), &payload)
                .await
        })
    }

    fn publish_steering<'a>(
        &'a self,
        msg: &'a SteeringMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.publish_json(format!("boxer.tasks.steer.{}", msg.session_id), msg)
                .await
        })
    }

    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.publish_json(format!("boxer.stream.{}", envelope.session_id), envelope)
                .await?;
            self.fanout_local(envelope).await;
            Ok(())
        })
    }

    fn publish_swarm_message<'a>(
        &'a self,
        msg: &'a SwarmMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let subject = match &msg.to {
                agents_protocol::SwarmTopic::Broadcast => {
                    format!("swarm.{}.broadcast", msg.session_id)
                }
                agents_protocol::SwarmTopic::DirectAgent(agent_id) => {
                    format!("swarm.{}.agent.{agent_id}", msg.session_id)
                }
            };
            self.publish_json(subject, msg).await
        })
    }

    fn subscribe_session_stream<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, broadcast::Receiver<EventEnvelope>> {
        Box::pin(async move {
            let mut map = self.channels.lock().await;
            map.entry(session_id.to_owned())
                .or_insert_with(|| broadcast::channel(RENDER_LANE_CAPACITY).0)
                .subscribe()
        })
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            if self.client.connection_state() == async_nats::connection::State::Disconnected {
                return Err(ServerError::Internal("nats disconnected".into()));
            }
            Ok(())
        })
    }
}

#[derive(Default)]
struct MemoryBusInner {
    task_requests: Vec<TaskRequest>,
    cancellations: Vec<(String, String, FencingToken)>,
    steering: Vec<SteeringMessage>,
    stream_events: Vec<EventEnvelope>,
    swarm_messages: Vec<SwarmMessage>,
    channels: HashMap<String, broadcast::Sender<EventEnvelope>>,
}

/// Deterministic in-memory message bus used for tests and standalone local runs.
#[derive(Clone, Default)]
pub struct InMemoryMessageBus {
    inner: Arc<Mutex<MemoryBusInner>>,
}

impl InMemoryMessageBus {
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns recorded task requests for assertions in integration tests.
    pub async fn recorded_task_requests(&self) -> Vec<TaskRequest> {
        self.inner.lock().await.task_requests.clone()
    }

    /// Returns recorded steering messages for assertions in integration tests.
    pub async fn recorded_steering(&self) -> Vec<SteeringMessage> {
        self.inner.lock().await.steering.clone()
    }

    /// Returns recorded cancellation requests.
    pub async fn recorded_cancellations(&self) -> Vec<(String, String, FencingToken)> {
        self.inner.lock().await.cancellations.clone()
    }
}

impl MessageBus for InMemoryMessageBus {
    fn publish_task_request<'a>(
        &'a self,
        req: &'a TaskRequest,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.task_requests.push(req.clone());
            Ok(())
        })
    }

    fn publish_task_cancel<'a>(
        &'a self,
        task_id: &'a str,
        session_id: &'a str,
        fencing_token: FencingToken,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.cancellations.push((
                task_id.to_owned(),
                session_id.to_owned(),
                fencing_token,
            ));
            Ok(())
        })
    }

    fn publish_steering<'a>(
        &'a self,
        msg: &'a SteeringMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.steering.push(msg.clone());
            Ok(())
        })
    }

    fn publish_stream_event<'a>(
        &'a self,
        envelope: &'a EventEnvelope,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            inner.stream_events.push(envelope.clone());
            let tx = inner
                .channels
                .entry(envelope.session_id.clone())
                .or_insert_with(|| broadcast::channel(RENDER_LANE_CAPACITY).0);
            let _ = tx.send(envelope.clone());
            Ok(())
        })
    }

    fn publish_swarm_message<'a>(
        &'a self,
        msg: &'a SwarmMessage,
    ) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async move {
            self.inner.lock().await.swarm_messages.push(msg.clone());
            Ok(())
        })
    }

    fn subscribe_session_stream<'a>(
        &'a self,
        session_id: &'a str,
    ) -> BoxFuture<'a, broadcast::Receiver<EventEnvelope>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().await;
            inner
                .channels
                .entry(session_id.to_owned())
                .or_insert_with(|| broadcast::channel(RENDER_LANE_CAPACITY).0)
                .subscribe()
        })
    }

    fn ping<'a>(&'a self) -> BoxFuture<'a, Result<(), ServerError>> {
        Box::pin(async { Ok(()) })
    }
}
