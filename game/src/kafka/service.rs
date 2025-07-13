use futures::lock::Mutex;
use rdkafka::config::RDKafkaLogLevel;
use rdkafka::consumer::{Consumer, StreamConsumer};
use rdkafka::message::{Message, OwnedMessage};
use rdkafka::producer::{FutureProducer, FutureRecord};
use rdkafka::{ClientConfig, Offset, TopicPartitionList};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::workflow::WorkflowDefinition;
use crate::workflow::service::WorkflowResource;

use super::topic::{KafkaTopic, WorkflowTopicMessage};

pub struct WorkflowsPublisher {
    producer: FutureProducer,
}

impl WorkflowsPublisher {
    pub async fn publish(&self, message: &WorkflowTopicMessage) -> Result<(), String> {
        let payload =
            serde_json::to_string(message).map_err(|e| format!("Serialization error: {}", e))?;

        let record = FutureRecord::to(KafkaTopic::Workflows.topic_name())
            .payload(Box::leak(payload.into_boxed_str()))
            .key("");

        let producer = self.producer.clone();
        let send_future = tokio::spawn(async move {
            producer
                .send(record, Duration::from_secs(5))
                .await
                .map_err(|(e, _)| format!("Failed to send message: {}", e))
        });

        send_future
            .await
            .map_err(|e| format!("Join error: {:?}", e))?
            .map(|_| ())
    }

    pub async fn create_workflow(&self, workflow: WorkflowResource) -> Result<(), String> {
        let message = WorkflowTopicMessage::Created { workflow };
        // println!("Published message: {:?}", message);

        let producer = self.producer.clone();
        let payload =
            serde_json::to_string(&message).map_err(|e| format!("Serialization error: {}", e))?;

        let record = FutureRecord::to(KafkaTopic::Workflows.topic_name())
            .payload(Box::leak(payload.into_boxed_str()))
            .key("");

        tokio::spawn(async move {
            if let Err(e) = producer
                .send(record, Duration::from_secs(5))
                .await
                .map_err(|(e, _)| format!("Failed to send message: {}", e))
            {
                eprintln!("Failed to publish workflow creation: {}", e);
            }
        });

        Ok(())
    }

    pub async fn update_workflow(&self, workflow: WorkflowResource) -> Result<(), String> {
        let message = WorkflowTopicMessage::Updated { workflow };
        // println!("Published message: {:?}", message);

        let producer = self.producer.clone();
        let payload =
            serde_json::to_string(&message).map_err(|e| format!("Serialization error: {}", e))?;

        let record = FutureRecord::to(KafkaTopic::Workflows.topic_name())
            .payload(Box::leak(payload.into_boxed_str()))
            .key("");

        tokio::spawn(async move {
            if let Err(e) = producer
                .send(record, Duration::from_secs(5))
                .await
                .map_err(|(e, _)| format!("Failed to send message: {}", e))
            {
                eprintln!("Failed to publish workflow creation: {}", e);
            }
        });

        Ok(())
    }

    pub async fn request_server_action_request(
        &self,
        id: String,
        workflow: WorkflowResource,
        action_id: String,
    ) -> Result<(), String> {
        let message = WorkflowTopicMessage::ServerActionRequest {
            id,
            workflow,
            action_id,
        };

        self.publish(&message).await
    }
}

// Main Kafka service
pub struct KafkaService {
    brokers: String,
    producer: FutureProducer,
    pub workflows: WorkflowsPublisher,
}

impl std::fmt::Debug for KafkaService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KafkaService")
            .field("brokers", &self.brokers)
            .finish()
    }
}

impl KafkaService {
    pub fn new(brokers: &str) -> Self {
        let producer: FutureProducer = ClientConfig::new()
            .set("bootstrap.servers", brokers)
            .set("message.timeout.ms", "5000")
            .set("security.protocol", "PLAINTEXT")
            .create()
            .expect("Failed to create Kafka producer");

        Self {
            brokers: brokers.to_string(),
            producer: producer.clone(),
            workflows: WorkflowsPublisher {
                producer: producer.clone(),
            },
        }
    }

    pub async fn start_workflow_consumer<F>(&self, group_id: String, handler: F)
    where
        F: Fn(
                WorkflowTopicMessage,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>
            + Send
            + Sync
            + Clone
            + 'static,
    {
        let brokers = self.brokers.clone();
        let handler_clone = handler.clone();

        tokio::spawn(async move {
            let consumer: StreamConsumer = ClientConfig::new()
                .set("group.id", &group_id)
                .set("bootstrap.servers", &brokers)
                .set("security.protocol", "PLAINTEXT")
                .set("auto.offset.reset", "latest")
                .set_log_level(RDKafkaLogLevel::Debug)
                .create()
                .expect("Failed to create consumer");

            let topic = KafkaTopic::Workflows.topic_name();
            let topics = &[topic];

            consumer
                .subscribe(topics)
                .expect("Failed to subscribe to topics");

            let mut tpl = TopicPartitionList::new();
            tpl.add_partition_offset(topic, 0, Offset::End)
                .expect("Failed to set partition offset");
            consumer.assign(&tpl).expect("Failed to assign partitions");

            loop {
                match consumer.recv().await {
                    Ok(msg) => {
                        if let Some(payload) = msg.payload() {
                            match serde_json::from_slice::<WorkflowTopicMessage>(payload) {
                                Ok(chat_message) => {
                                    let future = handler_clone(chat_message);
                                    future.await;
                                }
                                Err(e) => {
                                    eprintln!("Failed to deserialize workflow message: {}", e)
                                }
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("Kafka error: {:?}", err);
                    }
                }
            }
        });
    }
}

impl Clone for KafkaService {
    fn clone(&self) -> Self {
        Self {
            brokers: self.brokers.clone(),
            producer: self.producer.clone(),
            workflows: WorkflowsPublisher {
                producer: self.producer.clone(),
            },
        }
    }
}
