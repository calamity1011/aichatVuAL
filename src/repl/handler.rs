use crate::client::ChatGptClient;
use crate::config::SharedConfig;
use crate::render::render_stream;
use crate::utils::dump;

use anyhow::Result;
use crossbeam::channel::{unbounded, Sender};
use crossbeam::sync::WaitGroup;
use std::cell::RefCell;
use std::fs::File;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread::spawn;

pub enum ReplCmd {
    Submit(String),
    SetRole(String),
    UpdateConfig(String),
    Prompt(String),
    ClearRole,
    Info,
}

pub struct ReplCmdHandler {
    client: ChatGptClient,
    config: SharedConfig,
    state: RefCell<ReplCmdHandlerState>,
    ctrlc: Arc<AtomicBool>,
}

pub struct ReplCmdHandlerState {
    reply: String,
    save_file: Option<File>,
}

impl ReplCmdHandler {
    pub fn init(client: ChatGptClient, config: SharedConfig) -> Result<Self> {
        let save_file = config.as_ref().borrow().open_message_file()?;
        let ctrlc = Arc::new(AtomicBool::new(false));
        let state = RefCell::new(ReplCmdHandlerState {
            save_file,
            reply: String::new(),
        });
        Ok(Self {
            client,
            config,
            state,
            ctrlc,
        })
    }

    pub fn handle(&self, cmd: ReplCmd) -> Result<()> {
        match cmd {
            ReplCmd::Submit(input) => {
                if input.is_empty() {
                    self.state.borrow_mut().reply.clear();
                    return Ok(());
                }
                let prompt = self.config.borrow().get_prompt();
                let wg = WaitGroup::new();
                let highlight = self.config.borrow().highlight;
                let mut stream_handler = if highlight {
                    let (tx, rx) = unbounded();
                    let ctrlc = self.ctrlc.clone();
                    let wg = wg.clone();
                    spawn(move || {
                        let _ = render_stream(rx, ctrlc);
                        drop(wg);
                    });
                    ReplyStreamHandler::new(Some(tx), self.ctrlc.clone())
                } else {
                    ReplyStreamHandler::new(None, self.ctrlc.clone())
                };
                self.client
                    .send_message_streaming(&input, prompt, &mut stream_handler)?;
                let buffer = stream_handler.get_buffer();
                self.config.borrow().save_message(
                    self.state.borrow_mut().save_file.as_mut(),
                    &input,
                    buffer,
                )?;
                wg.wait();
                self.state.borrow_mut().reply = buffer.to_string();
            }
            ReplCmd::SetRole(name) => {
                let output = self.config.borrow_mut().change_role(&name);
                dump(output.trim(), 2);
            }
            ReplCmd::ClearRole => {
                self.config.borrow_mut().role = None;
                dump("Done", 2);
            }
            ReplCmd::Prompt(prompt) => {
                let output = self.config.borrow_mut().create_temp_role(&prompt);
                dump(output.trim(), 2);
            }
            ReplCmd::Info => {
                let output = self.config.borrow().info()?;
                dump(output.trim(), 2);
            }
            ReplCmd::UpdateConfig(input) => {
                let output = self.config.borrow_mut().update(&input)?;
                dump(output.trim(), 2);
            }
        }
        Ok(())
    }

    pub fn get_reply(&self) -> String {
        self.state.borrow().reply.to_string()
    }

    pub fn get_ctrlc(&self) -> Arc<AtomicBool> {
        self.ctrlc.clone()
    }
}

pub struct ReplyStreamHandler {
    sender: Option<Sender<ReplyStreamEvent>>,
    buffer: String,
    ctrlc: Arc<AtomicBool>,
}

impl ReplyStreamHandler {
    pub fn new(sender: Option<Sender<ReplyStreamEvent>>, ctrlc: Arc<AtomicBool>) -> Self {
        Self {
            sender,
            ctrlc,
            buffer: String::new(),
        }
    }

    pub fn text(&mut self, text: &str) {
        match self.sender.as_ref() {
            Some(tx) => {
                let _ = tx.send(ReplyStreamEvent::Text(text.to_string()));
            }
            None => {
                dump(text, 0);
            }
        }
        self.buffer.push_str(text);
    }

    pub fn done(&mut self) {
        match self.sender.as_ref() {
            Some(tx) => {
                let _ = tx.send(ReplyStreamEvent::Done);
            }
            None => {
                dump("", 2);
            }
        }
    }

    pub fn get_buffer(&self) -> &str {
        &self.buffer
    }

    pub fn get_ctrlc(&self) -> Arc<AtomicBool> {
        self.ctrlc.clone()
    }
}

pub enum ReplyStreamEvent {
    Text(String),
    Done,
}