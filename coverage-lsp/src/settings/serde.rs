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
** Created on: 2026-03-07T13:37:51
** Author: Sylvain Fargier <fargier.sylvain@gmail.com>
*/

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tower_lsp::lsp_types::Color;

pub fn serialize<S, V>(instant: &V, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    for<'a> Serde<&'a V>: Serialize,
{
    Serde(instant).serialize(serializer)
}

pub fn deserialize<'de, D, V>(deserializer: D) -> Result<V, D::Error>
where
    D: Deserializer<'de>,
    Serde<V>: Deserialize<'de>,
{
    Serde::deserialize(deserializer).map(|v| v.0)
}

struct Wrapper<T>(T);
impl<'de> Deserialize<'de> for Wrapper<Color> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        csscolorparser::Color::deserialize(deserializer).map(|c| {
            Wrapper(Color {
                red: c.r,
                green: c.g,
                blue: c.b,
                alpha: c.a,
            })
        })
    }
}

impl Serialize for Wrapper<&Color> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(
            csscolorparser::Color::new(self.0.red, self.0.green, self.0.blue, self.0.alpha)
                .to_string()
                .as_str(),
        )
    }
}

pub struct Serde<T>(T);
impl<'de, T> Deserialize<'de> for Serde<T>
where
    Wrapper<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Wrapper::deserialize(deserializer).map(|v| Serde(v.0))
    }
}

impl<T> Serialize for Serde<T>
where
    for<'a> Wrapper<&'a T>: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Wrapper(&self.0).serialize(serializer)
    }
}

impl<'de, T> Deserialize<'de> for Serde<Option<T>>
where
    Wrapper<T>: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<Wrapper<T>>::deserialize(deserializer).map(|o| Serde(o.map(|v| v.0)))
    }
}

impl<'a, T> Serialize for Serde<&'a Option<T>>
where
    Wrapper<&'a T>: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.0.as_ref().map(Wrapper).serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use tower_lsp::lsp_types::Color;

    #[test]
    fn serde() -> serde_json::Result<()> {
        #[derive(Serialize, Deserialize)]
        struct Test {
            #[serde(with = "super")]
            color: Option<Color>,
        }
        let ref_color = Color {
            red: 0.0,
            green: 0.0,
            blue: 1.0,
            alpha: 1.0,
        };

        let test: Test = serde_json::from_str(r#"{ "color": "blue" }"#)?;
        assert_eq!(test.color, Some(ref_color));
        let test: Test = serde_json::from_str(serde_json::to_string(&test)?.as_str())?;
        assert_eq!(test.color, Some(ref_color));

        assert_eq!(
            serde_json::from_str::<Test>(r#"{ "color": null }"#)?.color,
            None
        );
        assert!(
            serde_json::from_str::<Test>(r##"{ "color": "#11223344" }"##)?
                .color
                .is_some()
        );
        Ok(())
    }
}
