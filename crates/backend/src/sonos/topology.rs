//! ZoneGroupTopology `GetZoneGroupState` parsing.

use super::description::valid_player_uuid;

/// One player in a Sonos group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyMember {
    pub uuid: String,
    /// Device description URL.
    pub location: String,
    pub zone_name: String,
    /// Hidden players: the secondary speaker of a stereo pair, subs and
    /// surrounds. They are never zones of their own.
    pub invisible: bool,
}

impl TopologyMember {
    /// `http://ip:1400` derived from the description URL.
    pub fn base_url(&self) -> Option<String> {
        base_url_of(&self.location)
    }
}

/// A Sonos group: one coordinator and its members (coordinator first).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopologyGroup {
    pub id: String,
    pub coordinator: String,
    pub members: Vec<TopologyMember>,
}

impl TopologyGroup {
    /// Members that are rooms in their own right (stereo pair secondaries
    /// and home theatre satellites are left out).
    pub fn visible_members(&self) -> impl Iterator<Item = &TopologyMember> {
        self.members.iter().filter(|member| !member.invisible)
    }

    /// Roon-style group name: "Living Room + Kitchen".
    pub fn display_name(&self) -> String {
        let names: Vec<&str> = self
            .visible_members()
            .map(|member| member.zone_name.as_str())
            .collect();
        names.join(" + ")
    }
}

/// `http://host:port` from a URL like `http://host:port/xml/...`.
pub fn base_url_of(location: &str) -> Option<String> {
    let rest = location.strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    (!authority.is_empty()).then(|| format!("http://{authority}"))
}

/// Parses the `ZoneGroupState` value. Newer firmware wraps the groups in
/// `<ZoneGroupState><ZoneGroups>`, older firmware sends `<ZoneGroups>` alone.
pub fn parse_zone_group_state(document: &str) -> Result<Vec<TopologyGroup>, String> {
    let document = roxmltree::Document::parse(document).map_err(|error| error.to_string())?;
    let mut groups = Vec::new();
    for group in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "ZoneGroup")
    {
        let Some(coordinator) = group
            .attribute("Coordinator")
            .filter(|uuid| valid_player_uuid(uuid))
        else {
            continue;
        };
        let mut members: Vec<TopologyMember> = group
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "ZoneGroupMember")
            .filter_map(|member| {
                let uuid = member
                    .attribute("UUID")
                    .filter(|uuid| valid_player_uuid(uuid))?;
                let location = member
                    .attribute("Location")
                    .filter(|location| location.starts_with("http://"))?;
                Some(TopologyMember {
                    uuid: uuid.to_string(),
                    location: location.to_string(),
                    zone_name: member.attribute("ZoneName").unwrap_or(uuid).to_string(),
                    invisible: member.attribute("Invisible") == Some("1"),
                })
            })
            .collect();
        if !members.iter().any(|member| member.uuid == coordinator) {
            continue;
        }
        members.sort_by_key(|member| member.uuid != coordinator);
        groups.push(TopologyGroup {
            id: group.attribute("ID").unwrap_or(coordinator).to_string(),
            coordinator: coordinator.to_string(),
            members,
        });
    }
    groups.sort_by_key(|group| group.display_name());
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::{base_url_of, parse_zone_group_state};

    #[test]
    fn parses_groups_stereo_pairs_and_satellites() {
        let state = r#"<ZoneGroupState><ZoneGroups>
<ZoneGroup Coordinator="RINCON_LR01400" ID="RINCON_LR01400:58">
  <ZoneGroupMember UUID="RINCON_LR01400" Location="http://192.168.1.20:1400/xml/device_description.xml" ZoneName="Living Room" ChannelMapSet="RINCON_LR01400:LF,LF;RINCON_LR201400:RF,RF"/>
  <ZoneGroupMember UUID="RINCON_LR201400" Location="http://192.168.1.21:1400/xml/device_description.xml" ZoneName="Living Room" Invisible="1"/>
  <ZoneGroupMember UUID="RINCON_KI01400" Location="http://192.168.1.22:1400/xml/device_description.xml" ZoneName="Kitchen"/>
</ZoneGroup>
<ZoneGroup Coordinator="RINCON_TV01400" ID="RINCON_TV01400:3">
  <ZoneGroupMember UUID="RINCON_TV01400" Location="http://192.168.1.30:1400/xml/device_description.xml" ZoneName="Den">
    <Satellite UUID="RINCON_SUB01400" Location="http://192.168.1.31:1400/xml/device_description.xml" ZoneName="Den" Invisible="1"/>
  </ZoneGroupMember>
</ZoneGroup>
<ZoneGroup Coordinator="bogus" ID="x"><ZoneGroupMember UUID="bogus" Location="http://1.2.3.4:1400/" ZoneName="Bad"/></ZoneGroup>
</ZoneGroups><VanishedDevices/></ZoneGroupState>"#;
        let groups = parse_zone_group_state(state).expect("topology");
        assert_eq!(groups.len(), 2);
        let den = &groups[0];
        assert_eq!(den.display_name(), "Den");
        assert_eq!(den.members.len(), 1, "satellites are not group members");
        let living = &groups[1];
        assert_eq!(living.coordinator, "RINCON_LR01400");
        assert_eq!(living.members[0].uuid, "RINCON_LR01400");
        assert_eq!(living.display_name(), "Living Room + Kitchen");
        assert_eq!(living.visible_members().count(), 2);
        assert_eq!(
            living.members[0].base_url().as_deref(),
            Some("http://192.168.1.20:1400")
        );
    }

    #[test]
    fn accepts_legacy_unwrapped_zone_groups() {
        let legacy = r#"<ZoneGroups><ZoneGroup Coordinator="RINCON_A1" ID="RINCON_A1:1"><ZoneGroupMember UUID="RINCON_A1" Location="http://10.0.0.5:1400/xml/device_description.xml" ZoneName="Office"/></ZoneGroup></ZoneGroups>"#;
        let groups = parse_zone_group_state(legacy).expect("legacy topology");
        assert_eq!(groups[0].display_name(), "Office");
        assert_eq!(base_url_of("ftp://x"), None);
    }
}
