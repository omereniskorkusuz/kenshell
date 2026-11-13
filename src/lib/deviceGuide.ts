/**
 * What to switch on in the devices so the topology map can see the whole network:
 * LLDP (the links), read-only SNMP (discovery without logging in), sFlow / NetFlow
 * (end devices behind everything). Per CLI family, with this computer's address filled in.
 * Passwords are never filled in: the commands keep a placeholder for them.
 *
 * Device CLI is not translated; the titles are (`vendorFamily.*`).
 */

export interface GuideValues {
  /** This computer's address (where flows are sent). */
  collector: string;
  community: string;
  v3User: string;
}

export interface GuideBlock {
  /** i18n key under `guide.*` of the block's title. */
  what: 'lldp' | 'snmp2' | 'snmp3' | 'flow';
  lines: string[];
}

export interface GuideFamily {
  family: string;
  /** Some lines differ between releases: check them with `?` on the device. */
  verify: boolean;
  blocks: GuideBlock[];
}

const FAMILIES: GuideFamily[] = [
  {
    family: 'cisco',
    verify: false,
    blocks: [
      { what: 'lldp', lines: ['lldp run', 'cdp run'] },
      { what: 'snmp2', lines: ['snmp-server community {community} RO'] },
      {
        what: 'snmp3',
        lines: [
          'snmp-server group KENSHELL v3 priv',
          'snmp-server user {v3user} KENSHELL v3 auth sha <auth-password> priv aes 128 <priv-password>',
        ],
      },
      {
        what: 'flow',
        lines: [
          'flow exporter KENSHELL',
          ' destination {collector}',
          ' transport udp 2055',
          ' export-protocol netflow-v9',
          'flow monitor KENSHELL',
          ' exporter KENSHELL',
          ' record netflow ipv4 original-input',
          'interface <uplink>',
          ' ip flow monitor KENSHELL input',
        ],
      },
    ],
  },
  {
    family: 'ruijie',
    verify: true,
    blocks: [
      { what: 'lldp', lines: ['lldp enable'] },
      { what: 'snmp2', lines: ['snmp-server community {community} ro'] },
      {
        what: 'snmp3',
        lines: [
          'snmp-server view kenshell 1 include',
          'snmp-server group KENSHELL v3 priv read kenshell',
          'snmp-server user {v3user} KENSHELL v3 auth sha <auth-password> priv aes128 <priv-password>',
        ],
      },
      {
        what: 'flow',
        lines: [
          'sflow collector 1 destination {collector} 6343',
          'sflow sampling-rate 4096',
          'interface range GigabitEthernet 0/1-48',
          ' sflow enable ingress',
        ],
      },
    ],
  },
  {
    family: 'aruba',
    verify: true,
    blocks: [
      { what: 'lldp', lines: ['lldp'] },
      {
        what: 'snmp2',
        lines: ['snmp-server vrf default', 'snmp-server community {community}'],
      },
      {
        what: 'snmp3',
        lines: [
          'snmp-server vrf default',
          'snmpv3 user {v3user} auth sha auth-pass plaintext <auth-password> priv aes priv-pass plaintext <priv-password>',
        ],
      },
      {
        what: 'flow',
        lines: ['sflow', 'sflow collector {collector} port 6343', 'sflow sampling 4096'],
      },
    ],
  },
  {
    family: 'alcatel',
    verify: true,
    blocks: [
      {
        what: 'lldp',
        lines: [
          'lldp nearest-bridge chassis tlv management port-description enable system-name enable system-description enable system-capabilities enable',
          'lldp nearest-bridge chassis tlv management management-address enable',
        ],
      },
      {
        what: 'snmp2',
        lines: [
          'snmp security no-security',
          'user "kenshell-ro" password <password> read-only all no-auth',
          'snmp community-map "{community}" user "kenshell-ro" on',
        ],
      },
      {
        what: 'snmp3',
        lines: [
          'user "{v3user}" password <auth-password> read-only all sha+aes priv-password <priv-password>',
        ],
      },
      {
        what: 'flow',
        lines: [
          'sflow receiver 1 name KenShell address {collector} udp-port 6343',
          'sflow sampler 1 port 1/1/1-24 receiver 1 rate 2048',
        ],
      },
    ],
  },
  {
    family: 'juniper',
    verify: false,
    blocks: [
      { what: 'lldp', lines: ['set protocols lldp interface all'] },
      {
        what: 'snmp2',
        lines: ['set snmp community {community} authorization read-only'],
      },
      {
        what: 'snmp3',
        lines: [
          'set snmp v3 usm local-engine user {v3user} authentication-sha authentication-password <auth-password>',
          'set snmp v3 usm local-engine user {v3user} privacy-aes128 privacy-password <priv-password>',
          'set snmp v3 vacm security-to-group security-model usm security-name {v3user} group KENSHELL',
          'set snmp v3 vacm access group KENSHELL default-context-prefix security-model usm security-level privacy read-view all',
          'set snmp view all oid .1 include',
        ],
      },
      {
        what: 'flow',
        lines: [
          'set services flow-monitoring version9 template KENSHELL ipv4-template',
          'set forwarding-options sampling instance KENSHELL input rate 1000',
          'set forwarding-options sampling instance KENSHELL family inet output flow-server {collector} port 2055 version9 template KENSHELL',
          'set interfaces <interface> unit 0 family inet sampling input',
        ],
      },
    ],
  },
];

/** The guide with this network's values filled in (passwords stay placeholders). */
export function deviceGuide(v: GuideValues): GuideFamily[] {
  const fill = (line: string) =>
    line
      .replaceAll('{collector}', v.collector || '<this computer>')
      .replaceAll('{community}', v.community || '<community>')
      .replaceAll('{v3user}', v.v3User || '<user>');
  return FAMILIES.map((f) => ({
    ...f,
    blocks: f.blocks.map((b) => ({ ...b, lines: b.lines.map(fill) })),
  }));
}
