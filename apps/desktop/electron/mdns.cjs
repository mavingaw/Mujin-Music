// A small mDNS / DNS-SD browser for `_parson._tcp.local`, the service every
// Parson server advertises (crates/backend/src/discovery.rs). It lets the
// desktop app find servers on the network without starting its own server.
const dgram = require("node:dgram");

const SERVICE = "_parson._tcp.local";
const MDNS_ADDRESS = "224.0.0.251";
const MDNS_PORT = 5353;
const TYPE = Object.freeze({ A: 1, PTR: 12, TXT: 16, AAAA: 28, SRV: 33 });

function encodeName(name) {
  const parts = name.replace(/\.$/, "").split(".");
  const bytes = [];
  for (const part of parts) {
    const label = Buffer.from(part, "utf8");
    if (label.length > 63) throw new Error("DNS label too long");
    bytes.push(label.length, ...label);
  }
  bytes.push(0);
  return Buffer.from(bytes);
}

/** A PTR question for the Parson service. `unicast` sets the QU bit. */
function encodeQuery(service = SERVICE, { unicast = false } = {}) {
  const header = Buffer.alloc(12);
  header.writeUInt16BE(1, 4); // one question
  const question = Buffer.alloc(4);
  question.writeUInt16BE(TYPE.PTR, 0);
  question.writeUInt16BE(unicast ? 0x8001 : 0x0001, 2);
  return Buffer.concat([header, encodeName(service), question]);
}

function readName(buffer, start) {
  const labels = [];
  let offset = start;
  let end = -1;
  for (let jumps = 0; jumps < 32;) {
    if (offset >= buffer.length) throw new Error("Truncated DNS name");
    const length = buffer[offset];
    if (length === 0) {
      offset += 1;
      break;
    }
    if ((length & 0xc0) === 0xc0) {
      if (offset + 1 >= buffer.length) throw new Error("Truncated pointer");
      if (end < 0) end = offset + 2;
      offset = ((length & 0x3f) << 8) | buffer[offset + 1];
      jumps += 1;
      continue;
    }
    if (length > 63) throw new Error("Invalid DNS label");
    if (offset + 1 + length > buffer.length)
      throw new Error("Truncated DNS label");
    labels.push(buffer.toString("utf8", offset + 1, offset + 1 + length));
    offset += 1 + length;
  }
  return { name: labels.join("."), next: end >= 0 ? end : offset };
}

function parseTxt(data) {
  const properties = {};
  let offset = 0;
  while (offset < data.length) {
    const length = data[offset];
    const entry = data.toString("utf8", offset + 1, offset + 1 + length);
    offset += 1 + length;
    if (!entry) continue;
    const separator = entry.indexOf("=");
    const key = (separator < 0 ? entry : entry.slice(0, separator))
      .trim()
      .toLowerCase();
    if (key && !(key in properties)) {
      properties[key] = separator < 0 ? "" : entry.slice(separator + 1);
    }
  }
  return properties;
}

function ipv6(data) {
  const groups = [];
  for (let index = 0; index < 16; index += 2) {
    groups.push(data.readUInt16BE(index).toString(16));
  }
  return groups
    .join(":")
    .replace(/(^|:)0(:0)+(:|$)/, "::")
    .replace(/:{3,}/, "::");
}

/** Parses the answer, authority and additional records of a DNS message. */
function decodePacket(buffer) {
  if (!Buffer.isBuffer(buffer) || buffer.length < 12) {
    throw new Error("Not a DNS message");
  }
  const flags = buffer.readUInt16BE(2);
  const questions = buffer.readUInt16BE(4);
  const total =
    buffer.readUInt16BE(6) + buffer.readUInt16BE(8) + buffer.readUInt16BE(10);
  let offset = 12;
  for (let index = 0; index < questions; index += 1) {
    offset = readName(buffer, offset).next + 4;
  }
  const records = [];
  for (let index = 0; index < total && offset < buffer.length; index += 1) {
    const { name, next } = readName(buffer, offset);
    if (next + 10 > buffer.length) throw new Error("Truncated DNS record");
    const type = buffer.readUInt16BE(next);
    const ttl = buffer.readUInt32BE(next + 4);
    const length = buffer.readUInt16BE(next + 8);
    const start = next + 10;
    if (start + length > buffer.length) throw new Error("Truncated record");
    const data = buffer.subarray(start, start + length);
    const record = { name, type, ttl };
    if (type === TYPE.PTR) {
      record.target = readName(buffer, start).name;
    } else if (type === TYPE.SRV && length >= 7) {
      record.port = data.readUInt16BE(4);
      record.target = readName(buffer, start + 6).name;
    } else if (type === TYPE.TXT) {
      record.properties = parseTxt(data);
    } else if (type === TYPE.A && length === 4) {
      record.address = Array.from(data).join(".");
    } else if (type === TYPE.AAAA && length === 16) {
      record.address = ipv6(data);
    }
    records.push(record);
    offset = start + length;
  }
  return { response: Boolean(flags & 0x8000), records };
}

function sameName(left, right) {
  return (
    String(left).replace(/\.$/, "").toLowerCase() ===
    String(right).replace(/\.$/, "").toLowerCase()
  );
}

/**
 * Turns the records of one or more responses into Parson server candidates.
 * `fallbackAddress` is the responder's source address, used when a packet
 * names a host without carrying its A record.
 */
function servicesFromRecords(records, fallbackAddress = "") {
  const instances = new Set();
  for (const record of records) {
    if (record.type === TYPE.PTR && sameName(record.name, SERVICE)) {
      if (record.ttl > 0) instances.add(record.target);
    }
    if (
      record.type === TYPE.SRV &&
      String(record.name).toLowerCase().endsWith(`.${SERVICE}`)
    ) {
      if (record.ttl > 0) instances.add(record.name);
    }
  }
  const services = [];
  for (const instance of instances) {
    const srv = records.find(
      (record) => record.type === TYPE.SRV && sameName(record.name, instance),
    );
    if (!srv?.port) continue;
    const txt =
      records.find(
        (record) => record.type === TYPE.TXT && sameName(record.name, instance),
      )?.properties ?? {};
    if (txt.product && txt.product !== "parson-music") continue;
    const addresses = records
      .filter(
        (record) =>
          (record.type === TYPE.A || record.type === TYPE.AAAA) &&
          record.address &&
          sameName(record.name, srv.target),
      )
      .map((record) => record.address)
      .filter((address) => !address.startsWith("127.") && address !== "::1")
      .filter((address) => !/^fe80:/i.test(address));
    const ipv4 = addresses.filter((address) => !address.includes(":"));
    const host =
      ipv4[0] ??
      addresses[0] ??
      (fallbackAddress && !fallbackAddress.startsWith("127.")
        ? fallbackAddress
        : "");
    if (!host) continue;
    const label = instance.slice(0, -(SERVICE.length + 1));
    services.push({
      instance,
      instanceId: txt.id || "",
      name: txt.name || label.replace(/\\(.)/g, "$1") || "Parson",
      host,
      port: srv.port,
      origin: `http://${host.includes(":") ? `[${host}]` : host}:${srv.port}`,
      boot: txt.boot || "",
    });
  }
  return services;
}

/**
 * Browses for `durationMs`, calling `onService` for each server found (more
 * than once if it re-announces). Resolves with the de-duplicated list.
 * Uses two sockets: one joined to the mDNS group on 5353 (shared with any
 * system responder), and an ephemeral one that receives unicast replies when
 * 5353 is unavailable.
 */
function browse({
  durationMs = 3_000,
  onService = () => {},
  createSocket = dgram.createSocket,
} = {}) {
  return new Promise((resolve) => {
    const found = new Map();
    const sockets = [];
    const timers = [];
    let finished = false;
    const handle = (message, remote) => {
      let packet;
      try {
        packet = decodePacket(message);
      } catch {
        return;
      }
      if (!packet.response) return;
      for (const service of servicesFromRecords(
        packet.records,
        remote?.address,
      )) {
        const key = service.instanceId || service.origin;
        found.set(key, service);
        try {
          onService(service);
        } catch {}
      }
    };
    const query = encodeQuery();
    const queryUnicast = encodeQuery(SERVICE, { unicast: true });
    const open = (port, membership) => {
      const socket = createSocket({ type: "udp4", reuseAddr: true });
      sockets.push(socket);
      socket.on("error", () => {
        try {
          socket.close();
        } catch {}
      });
      socket.on("message", handle);
      socket.bind(port, () => {
        try {
          if (membership) {
            socket.addMembership(MDNS_ADDRESS);
            socket.setMulticastTTL(255);
          }
          socket.setMulticastLoopback(true);
        } catch {}
        const send = () => {
          if (finished) return;
          socket.send(
            membership ? query : queryUnicast,
            MDNS_PORT,
            MDNS_ADDRESS,
            () => {},
          );
        };
        send();
        // Re-ask a couple of times: multicast on Wi-Fi drops packets.
        timers.push(setTimeout(send, 600), setTimeout(send, 1_600));
      });
    };
    open(MDNS_PORT, true);
    open(0, false);
    timers.push(
      setTimeout(() => {
        finished = true;
        for (const timer of timers) clearTimeout(timer);
        for (const socket of sockets) {
          try {
            socket.close();
          } catch {}
        }
        resolve([...found.values()]);
      }, durationMs),
    );
  });
}

module.exports = {
  MDNS_ADDRESS,
  MDNS_PORT,
  SERVICE,
  TYPE,
  browse,
  decodePacket,
  encodeName,
  encodeQuery,
  servicesFromRecords,
};
