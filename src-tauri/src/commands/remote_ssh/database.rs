//! 远程 OceanBase 状态检查与 SQL 导入。

use super::*;

pub(crate) fn check_remote_oceanbase_status_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
) -> Result<OceanBaseHealth, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let (profile, database) = crate::commands::profile::require_database()?;
    let container = shell_quote(&profile.container_name(&database.service_key));
    let client = shell_quote(&database.client);
    let db_port = database.port;
    let script = format!(
        r#"set -e
STATE=$(docker inspect --format '{{{{.State.Status}}}}|{{{{if .State.Health}}}}{{{{.State.Health.Status}}}}{{{{else}}}}none{{{{end}}}}' {container} 2>/dev/null || echo "not_found|not_found")
echo "STATE=$STATE"
RUNNING=$(echo "$STATE" | cut -d'|' -f1)
if [ "$RUNNING" = "running" ]; then
  if docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -e 'SELECT 1;' >/dev/null 2>&1; then
    echo "READY=yes"
  else
    echo "READY=no"
  fi
else
  echo "READY=no"
fi
"#,
        container = container,
        client = client,
        db_port = db_port
    );
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(explain_ssh_failure(&stderr, key.as_deref()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut state = "not_found".to_string();
    let mut health = "not_found".to_string();
    let mut ready = false;
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("STATE=") {
            let mut parts = value.split('|');
            state = parts.next().unwrap_or("not_found").to_string();
            health = parts.next().unwrap_or("none").to_string();
        } else if let Some(value) = line.strip_prefix("READY=") {
            ready = value.trim() == "yes";
        }
    }

    let is_running = state == "running";
    Ok(OceanBaseHealth {
        is_running,
        is_ready: ready,
        container_health: health,
        message: if !is_running {
            "OceanBase 容器未运行或未启动".to_string()
        } else if ready {
            "OceanBase 数据库连接正常，已就绪可执行迁移".to_string()
        } else {
            "OceanBase 正在启动初始化中，请等待其变为 healthy 状态".to_string()
        },
    })
}

pub(crate) fn import_remote_oceanbase_sql_blocking(
    host: String,
    port: u16,
    user: String,
    key_path: Option<String>,
    sql_file_path: String,
) -> Result<DbImportResult, String> {
    let key = resolve_existing_key(key_path, &host)?;
    let (profile, database) = crate::commands::profile::require_database()?;
    let container = shell_quote(&profile.container_name(&database.service_key));
    let client = shell_quote(&database.client);
    let db_port = database.port;
    let database = database.name.clone();
    let script = format!(
        r#"set -e
SQL={}
if [ ! -f "$SQL" ]; then
  echo "远程 SQL 脚本不存在: $SQL" >&2
  exit 4
fi
docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -e "CREATE DATABASE IF NOT EXISTS {database} DEFAULT CHARACTER SET utf8mb4;"
docker exec -i {container} {client} -h127.0.0.1 -P{db_port} -uroot --default-character-set=utf8mb4 {database} < "$SQL"
echo "TABLE_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -N -e "SELECT count(*) FROM information_schema.tables WHERE table_schema='{database}';")"
echo "DICT_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot {database} -N -e 'SELECT count(*) FROM parameter_dict;' 2>/dev/null || echo 0)"
echo "USER_COUNT=$(docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot {database} -N -e 'SELECT count(*) FROM users;' 2>/dev/null || echo 0)"
docker exec {container} {client} -h127.0.0.1 -P{db_port} -uroot -N -e "SELECT table_name FROM information_schema.tables WHERE table_schema='{database}' ORDER BY table_name;" | while read -r table_name; do
  [ -n "$table_name" ] && echo "TABLE_NAME=$table_name"
done
"#,
        shell_quote(&sql_file_path),
        container = container,
        client = client,
        db_port = db_port,
        database = database
    );
    let output = run_ssh_script(&host, port, &user, key.as_deref(), &script)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        return Ok(DbImportResult {
            success: false,
            table_count: 0,
            dictionary_count: 0,
            user_count: 0,
            tables: Vec::new(),
            message: "远程 SQL 导入执行出错".to_string(),
            error: Some(if stderr.is_empty() {
                explain_ssh_failure("", key.as_deref())
            } else {
                stderr
            }),
        });
    }

    let mut table_count = 0usize;
    let mut dictionary_count = 0usize;
    let mut user_count = 0usize;
    let mut tables = Vec::new();
    for line in stdout.lines() {
        if let Some(value) = line.strip_prefix("TABLE_COUNT=") {
            table_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("DICT_COUNT=") {
            dictionary_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("USER_COUNT=") {
            user_count = value.trim().parse().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("TABLE_NAME=") {
            let name = value.trim();
            if !name.is_empty() {
                tables.push(name.to_string());
            }
        }
    }

    Ok(DbImportResult {
        success: true,
        table_count,
        dictionary_count,
        user_count,
        tables,
        message: format!(
            "数据库迁移完成！成功同步 {} 张数据表、{} 条字典数据与 {} 个系统用户",
            table_count, dictionary_count, user_count
        ),
        error: None,
    })
}
