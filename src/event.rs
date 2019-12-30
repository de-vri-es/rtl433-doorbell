use serde::Deserialize;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
pub struct Event {
	pub time: String,
	pub model: String,
	pub group: u32,
	pub unit: u32,
	pub id: u32,
	pub channel: u32,
	#[serde(deserialize_with = "deserialize_state")]
	pub state: bool,
}

fn deserialize_state<'de, D>(de: D) -> Result<bool, D::Error>
where
	D: serde::Deserializer<'de>
{
	let state : &str = Deserialize::deserialize(de)?;
	match state {
		"ON"  => Ok(true),
		"OFF" => Ok(false),
		x => Err(serde::de::Error::invalid_value(serde::de::Unexpected::Str(x), &"ON or OFF")),
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	fn test_decode() {
		const SAMPLE : &str = "{\"time\" : \"2019-12-11 15:21:51\", \"model\" : \"Proove-Security\", \"id\" : 3, \"channel\" : 4, \"state\" : \"ON\", \"unit\" : 2, \"group\" : 1}";
		let event = serde_json::from_str::<Event>(SAMPLE).expect("failed to parse JSON");
		assert_eq!(event.time, "2019-12-11 15:21:51");
		assert_eq!(event.model, "Proove-Security");
		assert_eq!(event.group, 1);
		assert_eq!(event.unit, 2);
		assert_eq!(event.id, 3);
		assert_eq!(event.channel, 4);
		assert_eq!(event.state, true);
	}
}
