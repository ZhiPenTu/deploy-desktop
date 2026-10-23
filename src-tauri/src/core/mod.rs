//! 跨功能共用的本机基础能力：配置目录、原子文件写入、跨进程锁、JSON 配置读写、
//! 备份替换、路径判断、Compose 读取与版本比较。
//!
//! 这里只放与具体业务无关的实现。命令层负责参数校验和把错误翻译成用户能处理的操作。
pub mod atomic_file;
pub mod backup;
pub mod compose;
pub mod config;
pub mod file_lock;
pub mod json_store;
pub mod path_guard;
pub mod version;
pub mod process;
pub mod shell;
