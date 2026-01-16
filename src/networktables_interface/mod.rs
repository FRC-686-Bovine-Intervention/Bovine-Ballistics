// Uses CLI code stolen from examples of the library usage because tokio doesn't like me
use std::{collections::HashMap, io::stdin};
use std::sync::{Arc, OnceLock};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;
use nt_client::{data::r#type::DataType, publish::GenericPublisher, Client, ClientHandle, NTAddr};
use nt_client::data::r#type::NetworkTableData;
use nt_client::subscribe::ReceivedMessage;
use tokio::{select, sync::{broadcast, mpsc}};
use tracing::Level;

static PREFIX: OnceLock<String> = OnceLock::new();

fn init_prefix(value: String) {
    PREFIX.set(value).expect("timestamp already initialized");
}

fn get_prefix() -> String {
    PREFIX.get().expect("timestamp not initialized yet").clone()
}
pub(crate) async fn run(local_test: bool, team_number: u16, aim_mode_path: String, current_pose_path: String, prefix: String) {
    init_prefix(prefix);

    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    let (cancel_send, mut cancel_recv) = broadcast::channel(1);

    let nt_addr: NTAddr;
    if local_test {
        println!("local_test is true");
        nt_addr = NTAddr::Local;
    } else {
        println!("local_test is false");
        nt_addr = NTAddr::TeamNumber(team_number);
    }
    let client = Client::new(Default::default());

    select! {
        // use exit here to forcibly stop the program without waiting for user input
        _ = cancel_recv.recv() => std::process::exit(0),
        res = client.connect_setup(|client| setup(client, cancel_send.clone())) => {
            let _ = cancel_send.send(());
            if let Err(err) = res {
                eprintln!("{err}");
            }
        },
    };
}

fn setup(client: &Client, cancel_send: broadcast::Sender<()>) {
    let mut cancel_recv = cancel_send.subscribe();

    let timestamp = Arc::new(AtomicI64::new(0));
    // create a map of unknown-type publishers
    // this prevents publishers from unpublishing
    let mut publishers = HashMap::new();

    // client handle in order to create topics
    let handle = client.handle().clone();

    let sub_topic = handle.topic("/AdvantageKit/Timestamp");
    tokio::spawn(async move {
        // subscribe to a topic to be able to publish
        // this is a bug within NetworkTables, see https://github.com/wpilibsuite/allwpilib/issues/7680
        let timestamp_sub = Arc::clone(&timestamp);
        let sub_task = tokio::spawn(async move {
            let mut sub = sub_topic.subscribe(Default::default()).await.unwrap();
            loop {
                match sub.recv().await {
                    Ok(ReceivedMessage::Announced(topic)) => println!("announced topic: {}", topic.name()),
                    Ok(ReceivedMessage::Updated((topic, value))) => {
                        let value = i64::from_value(&value).expect("updated value is a i64");
                        println!("topic {} updated to {value}", topic.name());
                        timestamp_sub.store(value, Ordering::Relaxed);
                    },
                    Ok(ReceivedMessage::Unannounced { name, .. }) => {
                        println!("topic {name} unannounced");
                    },
                    Ok(ReceivedMessage::UpdateProperties(topic)) => {
                        println!("topic {} updated its properties to {:?}", topic.name(), topic.properties());
                    }
                    Err(err) => {
                        eprint!("{err:?}");
                        break;
                    },
                }
            };
        });

        let (stdin_send, mut stdin_recv) = mpsc::channel(1);
        let timestamp_stdin = Arc::clone(&timestamp);
        // reads the next line in stdin
        // spawn blocking, since read_line blocks the main thread
        let stdin_task = tokio::task::spawn_blocking(move || {
            let mut init_commands_ran = false;
            let (mut x0, mut x1, mut x2) = (100,150,200);
            let _ = stdin_send.blocking_send(format!("publish int {}/Azimuth", get_prefix()));
            let _ = stdin_send.blocking_send(format!("publish int {}/Altitude", get_prefix()));
            let _ = stdin_send.blocking_send(format!("publish int {}/SurfaceSpeed", get_prefix()));
            let _ = stdin_send.blocking_send(format!("publish int {}/Timestamp", get_prefix()));
            loop {
                x0 += 1;
                x1 += 1;
                x2 += 1;
                let ts = timestamp_stdin.load(Ordering::Relaxed);
                let _ = stdin_send.blocking_send(format!("set 0 {}", x0));
                let _ = stdin_send.blocking_send(format!("set 1 {}", x1));
                let _ = stdin_send.blocking_send(format!("set 2 {}", x2));
                let _ = stdin_send.blocking_send(format!("set 3 {}", ts));
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        // handles user input
        let command_task = tokio::spawn(async move {
            loop {
                // read a command from the stdin
                let Some(command) = stdin_recv.recv().await else { break; };
                let mut segments = command.splitn(2, " ");
                let Some(command) = segments.next() else {
                    eprintln!("malformed command");
                    continue;
                };
                let args = segments.next().unwrap_or("");

                match command {
                    // display some helpful information
                    "help" => {
                        println!("NT 4.1 Publisher CLI");
                        println!("Commands:");
                        println!("- publish <type> <topic>");
                        println!("  publishes to <topic> with type <type>");
                        println!("  possible types: string, boolean, int");
                        println!();
                        println!("- set <id> <value>");
                        println!("  publish to <id> <value>");
                        println!();
                        println!("- unpublish <id>");
                        println!("  unpublishes <id>");
                    }
                    // publish a topic to the server
                    "publish" => match publish_command(args, &handle, &mut publishers).await {
                        Ok(id) => println!("publishing with id {id}"),
                        Err(CommandError(err)) => eprintln!("{err}"),
                    }
                    // publish to a topic
                    "set" => match set_command(args, &mut publishers).await {
                        Ok(()) => println!("successfully set topic"),
                        Err(CommandError(err)) => eprintln!("{err}"),
                    }
                    // unpublish
                    "unpublish" => match unpublish_command(args, &mut publishers).await {
                        Ok(()) => println!("successfully unpublished"),
                        Err(CommandError(err)) => eprintln!("{err}"),
                    }
                    _ => eprintln!("unknown command {command}"),
                };
            };
        });

        select! {
            _ = cancel_recv.recv() => {},
            _ = sub_task => {},
            _ = stdin_task => {},
            res = command_task => {
                if let Err(err) = res {
                    eprintln!("{err}");
                }
            },
        };
        let _ = cancel_send.send(());
    });
}

// publish <type> <topic>
async fn publish_command(
    args: &str,
    handle: &ClientHandle,
    publishers: &mut HashMap<i32, GenericPublisher>,
) -> Result<i32, CommandError> {
    let mut args = args.split_whitespace();

    // the first arg will be the type
    let r#type = args.next().ok_or(CommandError("missing type arg".to_string()))?;
    // the rest will be the topic, joined by spaces
    let topic_name = args.collect::<Vec<&str>>().join(" ");
    if topic_name.is_empty() { return Err(CommandError("missing topic arg".to_string())); };

    // support strings, booleans, and integers
    let r#type = match r#type {
        "string" => DataType::String,
        "boolean" => DataType::Boolean,
        "int" => DataType::Int,
        _ => return Err(CommandError(format!("unknown type {type}"))),
    };

    let topic = handle.topic(topic_name.clone());
    let publisher = topic.generic_publish(r#type, Default::default()).await?;

    let l0 = format!("{}/Azimuth", get_prefix());
    let l1 = format!("{}/Altitude", get_prefix());
    let l2 = format!("{}/SurfaceSpeed", get_prefix());
    let l3 = format!("{}/Timestamp", get_prefix());
    let id = match topic_name {
        l if l == l0 => 0,
        l if l == l1 => 1,
        l if l == l2 => 2,
        l if l == l3 => 3,
        _ => return Err(CommandError(format!("nonexistent publisher"))),
    };
    publishers.insert(id, publisher);
    Ok(id)
}

// set <id> <value>
async fn set_command(
    args: &str,
    publishers: &mut HashMap<i32, GenericPublisher>
) -> Result<(), CommandError> {
    // split the command args by whitespace
    let mut args = args.split_whitespace();

    // the first arg will be the id
    let id = args.next()
        .ok_or(CommandError("missing id arg".to_string()))?
        .parse::<i32>()?;
    // the rest will be the value, joined by spaces
    let value = args.collect::<Vec<&str>>().join(" ");
    if value.is_empty() { return Err(CommandError("missing topic arg".to_string())); };

    let publisher = publishers.get(&id).ok_or(CommandError("no publisher found".to_string()))?;

    // parse value as either a string, boolean, or integer and publish that value to the server
    match publisher.data_type() {
        DataType::String => publisher.set(value).await?,
        DataType::Boolean => publisher.set(value.parse::<bool>()?).await?,
        DataType::Int => publisher.set(value.parse::<i64>()?).await?,
        r#type => return Err(CommandError(format!("unsupported data type {type:?}"))),
    };

    Ok(())
}

// unpublish <id>
async fn unpublish_command(
    args: &str,
    publishers: &mut HashMap<i32, GenericPublisher>
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

struct CommandError(String);

impl<T: ToString> From<T> for CommandError {
    fn from(value: T) -> Self {
        Self(value.to_string())
    }
}
