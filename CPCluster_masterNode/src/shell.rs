use std::sync::Arc;
use tokio::runtime::Handle;

use cpcluster_common::{Task, TaskResult};
use uuid::Uuid;

use crate::state::{save_state, MasterNode};

async fn submit_task_and_wait(
    master: &MasterNode,
    task: Task,
    timeout_ms: u64,
) -> Option<TaskResult> {
    let id = Uuid::new_v4().to_string();
    master.pending_tasks.lock().await.insert(id.clone(), task);
    save_state(master).await;
    let start = std::time::Instant::now();
    loop {
        if let Some(result) = master.completed_tasks.lock().await.remove(&id) {
            save_state(master).await;
            return Some(result);
        }
        if start.elapsed().as_millis() as u64 > timeout_ms {
            master.pending_tasks.lock().await.remove(&id);
            save_state(master).await;
            return None;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

pub fn run_shell(master: Arc<MasterNode>, rt: Handle) {
    use std::io::{self, BufRead, Write};
    println!("CPCluster shell ready.\nCommands:\n  nodes                - list connected nodes with their role\n  tasks                - list active and pending tasks\n  task <id>           - show status or result of a task\n  addtask <type> <arg> - queue a task (type: compute|http)\n  getglobalram         - show memory usage of a worker\n  getstorage           - show disk usage of a disk node\n  exit                 - quit");
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    print!("> ");
    let _ = io::stdout().flush();
    let mut exit_shell = false;
    while let Some(Ok(line)) = lines.next() {
        for cmd in line.split(';') {
            match cmd.trim() {
                "nodes" => {
                    let nodes = master.connected_nodes.blocking_lock();
                    if nodes.is_empty() {
                        println!("No connected nodes");
                    } else {
                        for (addr, info) in nodes.iter() {
                            println!("{} ({:?})", addr, info.role);
                        }
                    }
                }
                "tasks" => {
                    let nodes = master.connected_nodes.blocking_lock();
                    let mut printed_active = false;
                    for (addr, info) in nodes.iter() {
                        for (id, task) in info.active_tasks.iter() {
                            println!("{}: {} -> {:?}", addr, id, task);
                            printed_active = true;
                        }
                    }
                    if !printed_active {
                        println!("No active tasks");
                    }
                    drop(nodes);
                    let pending = master.pending_tasks.blocking_lock();
                    if pending.is_empty() {
                        println!("No pending tasks");
                    } else {
                        println!("Pending tasks:");
                        for (id, task) in pending.iter() {
                            println!("{} -> {:?}", id, task);
                        }
                    }
                    drop(pending);
                    let completed = master.completed_tasks.blocking_lock();
                    if completed.is_empty() {
                        println!("No completed tasks");
                    } else {
                        println!("Completed tasks:");
                        for (id, res) in completed.iter() {
                            println!("{} -> {:?}", id, res);
                        }
                    }
                }
                cmd if cmd.trim_start().starts_with("task") => {
                    let parts: Vec<&str> = cmd.split_whitespace().collect();
                    if parts.len() != 2 {
                        println!("Usage: task <id>");
                    } else {
                        let id = parts[1];
                        if master.pending_tasks.blocking_lock().contains_key(id) {
                            println!("Task {} is pending", id);
                        } else if master
                            .connected_nodes
                            .blocking_lock()
                            .values()
                            .any(|n| n.active_tasks.contains_key(id))
                        {
                            println!("Task {} is running", id);
                        } else if let Some(result) = master.completed_tasks.blocking_lock().get(id)
                        {
                            println!("Task {} finished: {:?}", id, result);
                        } else {
                            println!("Task {} not found", id);
                        }
                    }
                }
                cmd if cmd.trim_start().starts_with("addtask") => {
                    let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
                    if parts.len() < 3 {
                        println!("Usage: addtask compute <expression> | addtask http <url>");
                    } else {
                        let id = Uuid::new_v4().to_string();
                        let task = match parts[1] {
                            "compute" => Task::Compute {
                                expression: parts[2].to_string().into(),
                            },
                            "http" => Task::HttpRequest {
                                url: parts[2].to_string(),
                            },
                            _ => {
                                println!("Unknown task type. Use 'compute' or 'http'.");
                                continue;
                            }
                        };
                        master
                            .pending_tasks
                            .blocking_lock()
                            .insert(id.clone(), task);
                        rt.block_on(save_state(&master));
                        println!("Queued task {}", id);
                    }
                }
                "getglobalram" => {
                    let has_worker = master
                        .connected_nodes
                        .blocking_lock()
                        .values()
                        .any(|n| matches!(n.role, cpcluster_common::NodeRole::Worker));
                    if !has_worker {
                        println!("No worker nodes available");
                    } else {
                        match rt.block_on(submit_task_and_wait(&master, Task::GetGlobalRam, 5000)) {
                            Some(TaskResult::Response(r)) => println!("{}", r.trim()),
                            Some(TaskResult::Error(e)) => println!("Error: {}", e),
                            _ => println!("Failed to retrieve RAM stats"),
                        }
                    }
                }
                "getstorage" => {
                    let has_disk = master
                        .connected_nodes
                        .blocking_lock()
                        .values()
                        .any(|n| matches!(n.role, cpcluster_common::NodeRole::Disk));
                    if !has_disk {
                        println!("No disk nodes available");
                    } else {
                        match rt.block_on(submit_task_and_wait(&master, Task::GetStorage, 5000)) {
                            Some(TaskResult::Response(r)) => println!("{}", r.trim()),
                            Some(TaskResult::Error(e)) => println!("Error: {}", e),
                            _ => println!("Failed to retrieve storage stats"),
                        }
                    }
                }
                "exit" | "quit" => {
                    println!("Exiting shell");
                    exit_shell = true;
                    break;
                }
                "" => {}
                _ => println!("Unknown command"),
            }
            if exit_shell {
                break;
            }
        }
        if exit_shell {
            break;
        }
        print!("> ");
        let _ = io::stdout().flush();
    }
}
