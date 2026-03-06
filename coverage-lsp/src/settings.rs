/*
** Copyright (C) 2026 Sylvain Fargier
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
** Created on: 2026-03-06T17:28:54
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use std::sync::{LazyLock, RwLock};

use serde_json::Value;
use tower_lsp::lsp_types::Color;
use regex::Regex;

pub static LSP_SETTINGS: RwLock<Settings> = RwLock::new(Settings::new());
static COLOR_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new("#([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})([0-9a-fA-F]{2})?").unwrap());

#[derive(Debug)]
pub struct Settings {
    pub hit: Option<Color>,
    pub miss: Option<Color>,
}

impl Default for Settings {
    fn default() -> Self {
        Self::new()
    }
}

fn parse_color(value: &str) -> Option<Color> {
    COLOR_RE.captures(value).map(|captures| Color {
            red: captures.get(1).map(|v| u8::from_str_radix(v.as_str(), 16).unwrap() as f32 / 255f32).unwrap(),
            green: captures.get(2).map(|v| u8::from_str_radix(v.as_str(), 16).unwrap() as f32 / 255f32).unwrap(),
            blue: captures.get(3).map(|v| u8::from_str_radix(v.as_str(), 16).unwrap() as f32 / 255f32).unwrap(),
            alpha: captures.get(4).map(|v| u8::from_str_radix(v.as_str(), 16).unwrap() as f32 / 255f32).unwrap_or(1.0),
        })
}

fn update_color(target: &mut Option<Color>, value: &Value) {
    match value {
        Value::String(value) => if let Some(color) = parse_color(value) { *target = Some(color); },
        Value::Null => *target = None,
        value => tracing::error!("invalid setting value: {value:?}")
    }
}

impl Settings {
    pub const fn new() -> Self {
        Self {
            hit: Some(Color {
                red: 0.0,
                green: 1.0,
                blue: 0.0,
                alpha: 0.1,
            }),
            miss: Some(Color {
                red: 1.0,
                green: 0.0,
                blue: 0.0,
                alpha: 0.1,
            }),
        }
    }

    pub fn update(&mut self, value: &Value) {
        if let Some(obj) = value.as_object() {
            if let Some(value) = obj.get("hit") {
                update_color(&mut self.hit, value);
            }
            if let Some(value) = obj.get("miss") {
                update_color(&mut self.miss, value);
            }
        }
    }
}

impl From<&Value> for Settings {
    fn from(value: &Value) -> Self {
        let mut settings = Settings::new();
        settings.update(value);
        settings
    }
}
