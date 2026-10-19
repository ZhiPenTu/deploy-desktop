use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OceanBaseHealth {
    pub is_running: bool,
    pub is_ready: bool,
    pub container_health: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbImportResult {
    pub success: bool,
    pub table_count: usize,
    pub dictionary_count: usize,
    pub user_count: usize,
    pub tables: Vec<String>,
    pub message: String,
    pub error: Option<String>,
}

#[tauri::command]
pub fn check_oceanbase_status() -> Result<OceanBaseHealth, String> {
    let profile = super::profile::deployment_profile();
    let container = profile.database_container();
    let client = profile.database.client.as_str();
    let port_arg = format!("-P{}", profile.database.port);

    // 检查容器运行状态
    let inspect_out = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{.State.Status}}|{{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}",
            &container,
        ])
        .output();

    let (is_running, container_health) = match inspect_out {
        Ok(out) if out.status.success() => {
            let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let parts: Vec<&str> = text.split('|').collect();
            let state = parts.first().unwrap_or(&"").to_string();
            let health = parts.get(1).unwrap_or(&"").to_string();
            (state == "running", health)
        }
        _ => (false, "not_found".to_string()),
    };

    if !is_running {
        return Ok(OceanBaseHealth {
            is_running: false,
            is_ready: false,
            container_health,
            message: "OceanBase 容器未运行或未启动".to_string(),
        });
    }

    // 探测数据库是否允许连接并响应 SELECT 1
    let ping_out = Command::new("docker")
        .args([
            "exec",
            &container,
            client,
            "-h127.0.0.1",
            &port_arg,
            "-uroot",
            "-e",
            "SELECT 1;",
        ])
        .output();

    let is_ready = match ping_out {
        Ok(out) => out.status.success(),
        _ => false,
    };

    let msg = if is_ready {
        "OceanBase 数据库连接正常，已就绪可执行迁移".to_string()
    } else {
        "OceanBase 正在启动初始化中，请等待其变为 healthy 状态...".to_string()
    };

    Ok(OceanBaseHealth {
        is_running: true,
        is_ready,
        container_health,
        message: msg,
    })
}

#[tauri::command]
pub fn import_oceanbase_sql(sql_file_path: String) -> Result<DbImportResult, String> {
    let profile = super::profile::deployment_profile();
    let container = profile.database_container();
    let client = profile.database.client.as_str();
    let port_arg = format!("-P{}", profile.database.port);
    let database = profile.database.name.as_str();
    let create_statement = format!(
        "CREATE DATABASE IF NOT EXISTS {} DEFAULT CHARACTER SET utf8mb4;",
        database
    );

    let p = Path::new(&sql_file_path);
    if !p.exists() {
        return Err(format!("SQL 脚本文件不存在: {}", p.display()));
    }

    // 1. 确保业务数据库已创建
    let create_db_cmd = Command::new("docker")
        .args([
            "exec",
            &container,
            client,
            "-h127.0.0.1",
            &port_arg,
            "-uroot",
            "-e",
            &create_statement,
        ])
        .output();

    if let Err(e) = create_db_cmd {
        return Err(format!("初始化业务数据库失败: {}", e));
    }

    // 2. 导入 SQL 脚本
    let file = File::open(p).map_err(|e| format!("打开 SQL 文件失败: {}", e))?;
    let import_child = Command::new("docker")
        .args([
            "exec",
            "-i",
            &container,
            client,
            "-h127.0.0.1",
            &port_arg,
            "-uroot",
            "--default-character-set=utf8mb4",
            database,
        ])
        .stdin(Stdio::from(file))
        .output();

    match import_child {
        Ok(out) if out.status.success() => {
            // 3. 校验导入后的数据一致性
            let (table_count, tables) = query_table_list();
            let dict_count = query_row_count("parameter_dict");
            let user_count = query_row_count("users");

            Ok(DbImportResult {
                success: true,
                table_count,
                dictionary_count: dict_count,
                user_count,
                tables,
                message: format!(
                    "数据库迁移完成！成功同步 {} 张数据表、{} 条字典数据与 {} 个系统用户",
                    table_count, dict_count, user_count
                ),
                error: None,
            })
        }
        Ok(out) => {
            let err_text = String::from_utf8_lossy(&out.stderr).to_string();
            Ok(DbImportResult {
                success: false,
                table_count: 0,
                dictionary_count: 0,
                user_count: 0,
                tables: Vec::new(),
                message: "SQL 导入执行出错".to_string(),
                error: Some(err_text),
            })
        }
        Err(e) => Err(format!("执行导入进程失败: {}", e)),
    }
}

fn query_table_list() -> (usize, Vec<String>) {
    let profile = super::profile::deployment_profile();
    let container = profile.database_container();
    let client = profile.database.client.as_str();
    let port_arg = format!("-P{}", profile.database.port);
    let query = format!(
        "SELECT table_name FROM information_schema.tables WHERE table_schema='{}' ORDER BY table_name;",
        profile.database.name
    );
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            client,
            "-h127.0.0.1",
            &port_arg,
            "-uroot",
            "-N",
            "-e",
            &query,
        ])
        .output();

    if let Ok(o) = out {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout);
            let tables: Vec<String> = text
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            return (tables.len(), tables);
        }
    }
    (0, Vec::new())
}

fn query_row_count(table_name: &str) -> usize {
    let profile = super::profile::deployment_profile();
    let container = profile.database_container();
    let client = profile.database.client.as_str();
    let port_arg = format!("-P{}", profile.database.port);
    let query = format!("SELECT count(*) FROM {};", table_name);
    let out = Command::new("docker")
        .args([
            "exec",
            &container,
            client,
            "-h127.0.0.1",
            &port_arg,
            "-uroot",
            &profile.database.name,
            "-N",
            "-e",
            &query,
        ])
        .output();

    if let Ok(o) = out {
        if o.status.success() {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            return text.parse::<usize>().unwrap_or(0);
        }
    }
    0
}
