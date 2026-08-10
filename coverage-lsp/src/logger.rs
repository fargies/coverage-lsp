/*
** Copyright (C) 2025 Sylvain Fargier
**
** This software is provided 'as-is', without any express or implied
** warranty.  In no event will the authors be held liable for any damages
** arising from the use of this software.
**
** Permission is granted to anyone to use this software for any purpose,
** including commercial applications, and to alter it and redistribute it
** freely, subject to the following restrictions:
**
** 1. The origin of this software must not be misrepresented; you must not
**    claim that you wrote the original software. If you use this software
**    in a product, an acknowledgment in the product documentation would be
**    appreciated but is not required.
** 2. Altered source versions must be plainly marked as such, and must not be
**    misrepresented as being the original software.
** 3. This notice may not be removed or altered from any source distribution.
**
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::sync::{Arc, Mutex, Weak};
use tokio::sync::mpsc;
use tower_lsp::lsp_types::MessageType;
use tracing::{
    Level, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    EnvFilter, Layer, Registry, layer::SubscriberExt, util::SubscriberInitExt,
};

use crate::coverage_lsp::CoverageLanguageServerContext;

static SERVICE: Mutex<Weak<CoverageLanguageServerContext>> = Mutex::new(Weak::new());

#[derive(Default)]
struct FieldsVisitor {
    // message is typically where tracing puts the format string argument
    message: Option<String>,
    // all other fields as key/value
    others: Vec<(String, String)>,
}

impl FieldsVisitor {
    pub fn size_hint(&self) -> usize {
        self.message.as_ref().map(|m| m.len()).unwrap_or(0)
            + self
                .others
                .iter()
                .map(|(name, value)| name.len() + value.len() + 2)
                .sum::<usize>()
    }
}

#[allow(clippy::to_string_trait_impl)]
impl ToString for FieldsVisitor {
    fn to_string(&self) -> String {
        let mut ret = String::with_capacity(self.size_hint());
        if let Some(msg) = self.message.as_ref() {
            ret.push_str(msg.as_str());
        }
        for (name, value) in self.others.iter() {
            ret.push(' ');
            ret.push_str(name.as_str());
            ret.push('=');
            ret.push_str(value.as_str());
        }
        ret
    }
}

impl Visit for FieldsVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let k = field.name().to_string();
        let v = format!("{value:?}");

        if k == "message" {
            self.message = Some(v);
        } else {
            self.others.push((k, v));
        }
    }

    // If you want Display when possible, you can add record_str / record_i64 etc,
    // but many values will come via record_debug anyway depending on field types.
    fn record_str(&mut self, field: &Field, value: &str) {
        let k = field.name().to_string();
        if k == "message" {
            self.message = Some(value.to_string());
        } else {
            self.others.push((k, value.to_string()));
        }
    }
}

pub fn init_logger() {
    let (tx, rx) = mpsc::channel::<LogMessage>(128);
    Registry::default()
        .with(
            EnvFilter::builder()
                .with_default_directive(Level::DEBUG.into())
                .from_env_lossy(),
        )
        .with(LogLayer { tx })
        .init();
    let pump = LogPump {
        context: Weak::new(),
        rx,
    };
    tokio::spawn(pump.run());
}

#[derive(Debug, Clone)]
struct LogMessage {
    pub level: MessageType,
    pub message: String,
}

pub struct LogLayer {
    tx: mpsc::Sender<LogMessage>,
}

impl<S> Layer<S> for LogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &tracing::Event, _ctx: tracing_subscriber::layer::Context<'_, S>) {
        let meta = event.metadata();
        let mut v = FieldsVisitor::default();
        event.record(&mut v);

        let level = match *meta.level() {
            tracing::Level::WARN => MessageType::WARNING,
            tracing::Level::ERROR => MessageType::ERROR,
            tracing::Level::INFO => MessageType::INFO,
            _ => MessageType::LOG,
        };

        let _ = self.tx.try_send(LogMessage {
            level,
            message: v.to_string(),
        });
    }
}

pub struct LogPump {
    rx: mpsc::Receiver<LogMessage>,
    context: Weak<CoverageLanguageServerContext>,
}

impl LogPump {

    pub fn set_service(service: Weak<CoverageLanguageServerContext>) {
        *SERVICE.lock().unwrap() = service;
    }

    pub async fn run(mut self) {
        while let Some(msg) = self.rx.recv().await {
            if let Some(ctx) = self.get_context() {
                ctx.client.log_message(msg.level, msg.message).await;
            }
        }
        tracing::warn!("stopping logger");
    }

    pub fn get_context(&mut self) -> Option<Arc<CoverageLanguageServerContext>> {
        if let Some(ctx) = self.context.upgrade() {
            Some(ctx)
        } else {
            self.context = SERVICE.lock().unwrap().clone();
            self.context.upgrade()
        }
    }
}
