use serde::{Deserialize, Serialize};

// A Docker event
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
