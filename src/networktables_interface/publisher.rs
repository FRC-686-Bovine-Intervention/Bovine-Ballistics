use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;
use nt_client::{Client, ClientHandle};
use nt_client::data::r#type::DataType;
use nt_client::publish::GenericPublisher;
use tokio::select;
use tokio::sync::{broadcast, Mutex};
use lazy_static::lazy_static;
use tracing::Level;
//
// lazy_static! {
//     static ref publishers: Arc<Mutex<HashMap<i32, GenericPublisher>>> = Arc::new(Mutex::new(HashMap::new()));
// }

pub async fn run(handle: ClientHandle) {
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let (cancel_send, mut cancel_recv) = broadcast::channel(1);

    setup(handle, cancel_send.clone());
    select! {
        // use exit here to forcibly stop the program without waiting for user input
        _ = cancel_recv.recv() => std::process::exit(0),
    };
}
fn setup(handle: ClientHandle, cancel_send: broadcast::Sender<()>) {
    let mut cancel_recv = cancel_send.subscribe();

    let sub_topic = handle.topic("/tmp");
    tokio::spawn(async move {
        // subscribe to a topic to be able to publish
        // this is a bug within NetworkTables, see https://github.com/wpilibsuite/allwpilib/issues/7680
        let sub_task = tokio::spawn(async move {
            let mut sub = sub_topic.subscribe(Default::default()).await.unwrap();
            loop {
                let _ = sub.recv().await;
            };
        });

        let handle_clone = handle.clone();

        select! {
            _ = cancel_recv.recv() => {},
            _ = sub_task => {},
        };
        let _ = cancel_send.send(());
    });
}

pub(crate) async fn publish_command(
    topic_name: &str,
    r#type: DataType,
    handle: &ClientHandle,
    publishers: &mut HashMap<i32, GenericPublisher>,
) -> Result<i32, CommandError> {
    let topic = handle.topic(topic_name);
    let publisher = topic.generic_publish(r#type, Default::default()).await?;
    let id = publisher.id();
    publishers.insert(id, publisher);
    Ok(id)
}
pub async fn set_command(
    id: i32,
    value: Box<dyn Any + Send>,
    publishers: &mut HashMap<i32, GenericPublisher>,
) -> Result<(), CommandError> {
    let publisher = publishers.get(&id).ok_or(CommandError("no publisher found".to_string()))?;

    // parse value as either a string, boolean, or integer and publish that value to the server
    match publisher.data_type() {
        DataType::Double => publisher.set(*value.downcast_ref::<f64>().unwrap()).await?,
        DataType::Int => publisher.set(*value.downcast_ref::<i64>().unwrap()).await?,
        r#type => return Err(CommandError(format!("unsupported data type {type:?}"))),
    };

    Ok(())
}

// unpublish <id>
async fn unpublish_command(
    args: &str,
    publishers: &mut HashMap<i32, GenericPublisher>,
) -> Result<(), CommandError> {
    // split the command args by whitespace
    let mut args = args.split_whitespace();

    // the first arg will be the id
    let id = args.next()
        .ok_or(CommandError("missing id arg".to_string()))?
        .parse::<i32>()?;

    // removing from the map causes the publisher to be owned by this function,

    // which will be dropped and unpublished by the end of this block
    publishers.remove(&id).ok_or(CommandError("publisher not found".to_string()))?;


    Ok(())
}

#[derive(Debug)]
pub struct CommandError(String);

impl<T: ToString> From<T> for CommandError {
    fn from(value: T) -> Self {
        Self(value.to_string())
    }
}