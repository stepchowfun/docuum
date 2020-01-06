use serde::{Deserialize, Serialize};

// A Docker event (a line of output from `docker events --format '{{json .}}'`)
#[derive(Deserialize, Serialize, Debug)]
pub struct Event {
    #[serde(rename = "Type")]
    pub r#type: String,

    #[serde(rename = "Action")]
    pub action: String,

    #[serde(rename = "Actor")]
    pub actor: EventActor,
}

// A Docker event actor
#[derive(Deserialize, Serialize, Debug)]
pub struct EventActor {
    #[serde(rename = "Attributes")]
    pub attributes: EventActorAttributes,
}

// Docker event actor attributes
#[derive(Deserialize, Serialize, Debug)]
pub struct EventActorAttributes {
    pub image: String,
}

// A line of output from `docker system df --format '{{json .}}'`
#[derive(Deserialize, Serialize, Debug)]
pub struct SpaceRecord {
    #[serde(rename = "Type")]
    pub r#type: String,

    #[serde(rename = "Size")]
    pub size: String,
}
