//! Ruijie commands whose layout matches the Cisco parser: `show vlan` (like
//! `show vlan brief`), `show ip route`, `show spanning-tree summary` (port table).

use super::super::{ip_route, spanning_tree, vlan_brief};
use crate::analyzer::ParserDef;

pub static VLAN: ParserDef = ParserDef {
    id: "ruijie.show_vlan",
    families: &["ruijie"],
    command: "show vlan",
    aliases: &[],
    columns: vlan_brief::PARSER.columns,
    parse: vlan_brief::parse,
};

pub static IP_ROUTE: ParserDef = ParserDef {
    id: "ruijie.show_ip_route",
    families: &["ruijie"],
    command: "show ip route",
    aliases: &[],
    columns: ip_route::PARSER.columns,
    parse: ip_route::parse,
};

pub static STP_SUMMARY: ParserDef = ParserDef {
    id: "ruijie.show_spanning_tree_summary",
    families: &["ruijie"],
    command: "show spanning-tree summary",
    aliases: &[],
    columns: spanning_tree::PARSER.columns,
    parse: spanning_tree::parse,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlan_fixture() {
        let rows = (VLAN.parse)(include_str!("../../fixtures/ruijie_show_vlan.txt"));
        assert_eq!(rows.len(), 3);
        assert_eq!(
            rows[0],
            [
                "1",
                "VLAN0001",
                "STATIC",
                "Gi0/2, Gi0/3, Gi0/4, Gi0/5, Gi0/6, Gi0/7"
            ]
        );
        assert_eq!(rows[2], ["100", "MGMT", "STATIC", "Ag1"]);
    }

    #[test]
    fn route_fixture() {
        let rows = (IP_ROUTE.parse)(include_str!("../../fixtures/ruijie_show_ip_route.txt"));
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[1], ["C", "10.0.0.0/24", "", "", "VLAN 10", ""]);
        assert_eq!(rows[2], ["C", "10.0.0.2/32", "", "", "", ""]);
        assert_eq!(
            rows[3],
            [
                "O IA",
                "172.16.0.0/16",
                "110/2",
                "10.0.0.3",
                "VLAN 10",
                "00:10:02"
            ]
        );
    }

    #[test]
    fn stp_fixture() {
        let rows = (STP_SUMMARY.parse)(include_str!(
            "../../fixtures/ruijie_show_spanning_tree_summary.txt"
        ));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1][1..5], ["Gi0/49", "Root", "FWD", "2000"]);
        assert_eq!(rows[0][7], "root", "`this bridge is root`");
    }
}
