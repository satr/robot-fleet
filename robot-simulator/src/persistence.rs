use std::{collections::HashSet, path::Path};

use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub(crate) async fn load_processed_commands(path: &Path) -> anyhow::Result<HashSet<Uuid>> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => Ok(contents
            .lines()
            .filter_map(|line| Uuid::parse_str(line).ok())
            .collect()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(err) => Err(err.into()),
    }
}

pub(crate) async fn persist_processed_command(path: &Path, command_id: Uuid) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await?;
    file.write_all(format!("{command_id}\n").as_bytes()).await?;
    file.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::{load_processed_commands, persist_processed_command};

    #[tokio::test]
    async fn processed_command_ids_are_persisted_and_loaded() {
        let command_id = Uuid::new_v4();
        let path = std::env::temp_dir().join(format!(
            "robot-simulator-processed-commands-{command_id}.txt"
        ));

        persist_processed_command(&path, command_id)
            .await
            .expect("persist command id");

        let processed = load_processed_commands(&path)
            .await
            .expect("load processed command ids");
        assert!(processed.contains(&command_id));

        tokio::fs::remove_file(path)
            .await
            .expect("remove processed commands test file");
    }
}
