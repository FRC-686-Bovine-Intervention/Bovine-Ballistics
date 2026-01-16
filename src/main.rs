use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task;
use crate::helpers::parse_json_to_config_struct;
use crate::web_interface::WebInterface;

mod web_interface;
mod calculator;
mod helpers;
mod networktables_interface;

#[tokio::main]
async fn main() {
    let settings = parse_json_to_config_struct("data/settings.json").unwrap();

    let web_worker: Arc<Mutex<WebInterface>> = Arc::new(Mutex::new(WebInterface::new(3000)));
    let web_task = task::spawn(async move {
        web_worker.lock().await.run().await;
    });
    let rio_task = task::spawn(async move {
         networktables_interface::run(settings.local_test, settings.team_number, settings.aim_mode_path, settings.current_pose_path, settings.prefix).await;
    });
    let _ = tokio::join!(web_task, rio_task);
}
