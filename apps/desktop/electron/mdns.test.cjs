const test = require("node:test");
const assert = require("node:assert/strict");
const { EventEmitter } = require("node:events");
const {
  SERVICE,
  TYPE,
  browse,
  decodePacket,
  encodeName,
  encodeQuery,
  servicesFromRecords,
} = require("./mdns.cjs");

function record(name, type, data, ttl = 120) {
  const header = Buffer.alloc(10);
  header.writeUInt16BE(type, 0);
  header.writeUInt16BE(0x8001, 2);
  header.writeUInt32BE(ttl, 4);
  header.writeUInt16BE(data.length, 8);
  return Buffer.concat([encodeName(name), header, data]);
}

function srv(port, target) {
  const head = Buffer.alloc(6);
  head.writeUInt16BE(port, 4);
  return Buffer.concat([head, encodeName(target)]);
}

function txt(entries) {
  return Buffer.concat(
    Object.entries(entries).map(([key, value]) => {
      const entry = Buffer.from(`${key}=${value}`);
      return Buffer.concat([Buffer.from([entry.length]), entry]);
    }),
  );
}

// A response shaped like the one mdns-sd sends for the Parson advertisement,
// including a compressed name pointer in the PTR answer.
function parsonResponse({
  instance = "Living room",
  id = "living-room",
  address = [192, 168, 1, 20],
  port = 1993,
  product = "parson-music",
} = {}) {
  const fullName = `${instance}.${SERVICE}`;
  const host = `parson-${id.slice(0, 8)}.local`;
  const header = Buffer.alloc(12);
  header.writeUInt16BE(0x8400, 2);
  header.writeUInt16BE(1, 6);
  header.writeUInt16BE(3, 10);
  const ptrName = encodeName(SERVICE); // at offset 12
  const ptrHead = Buffer.alloc(10);
  ptrHead.writeUInt16BE(TYPE.PTR, 0);
  ptrHead.writeUInt16BE(1, 2);
  ptrHead.writeUInt32BE(4500, 4);
  // Instance label followed by a pointer back to the service name.
  const instanceLabel = Buffer.from(instance);
  const ptrData = Buffer.concat([
    Buffer.from([instanceLabel.length]),
    instanceLabel,
    Buffer.from([0xc0, 12]),
  ]);
  ptrHead.writeUInt16BE(ptrData.length, 8);
  return Buffer.concat([
    header,
    ptrName,
    ptrHead,
    ptrData,
    record(fullName, TYPE.SRV, srv(port, host)),
    record(
      fullName,
      TYPE.TXT,
      txt({ id, name: instance, product, version: "1", boot: "abc123" }),
    ),
    record(host, TYPE.A, Buffer.from(address)),
  ]);
}

test("queries ask for the Mujin Music service by PTR", () => {
  const query = encodeQuery();
  assert.equal(query.readUInt16BE(4), 1);
  const packet = decodePacket(
    Buffer.concat([query.subarray(0, 12), query.subarray(12)]),
  );
  assert.equal(packet.response, false);
  assert.equal(
    encodeQuery(SERVICE, { unicast: true }).readUInt16BE(query.length - 2),
    0x8001,
  );
});

test("responses resolve to a connectable server", () => {
  const packet = decodePacket(parsonResponse());
  assert.equal(packet.response, true);
  assert.deepEqual(servicesFromRecords(packet.records), [
    {
      instance: `Living room.${SERVICE}`,
      instanceId: "living-room",
      name: "Living room",
      host: "192.168.1.20",
      port: 1993,
      origin: "http://192.168.1.20:1993",
      boot: "abc123",
    },
  ]);
});

test("a missing A record falls back to the responder's address", () => {
  const records = decodePacket(parsonResponse()).records.filter(
    (entry) => entry.type !== TYPE.A,
  );
  assert.equal(
    servicesFromRecords(records, "10.0.0.7")[0].origin,
    "http://10.0.0.7:1993",
  );
  assert.deepEqual(servicesFromRecords(records, "127.0.0.1"), []);
});

test("other products and goodbye packets are ignored", () => {
  assert.deepEqual(
    servicesFromRecords(
      decodePacket(parsonResponse({ product: "something-else" })).records,
    ),
    [],
  );
  const goodbye = decodePacket(parsonResponse()).records.map((entry) => ({
    ...entry,
    ttl: 0,
  }));
  assert.deepEqual(servicesFromRecords(goodbye), []);
});

test("malformed packets throw instead of looping", () => {
  assert.throws(() => decodePacket(Buffer.alloc(4)));
  const looping = Buffer.alloc(16);
  looping.writeUInt16BE(0x8400, 2);
  looping.writeUInt16BE(1, 6);
  looping.writeUInt8(0xc0, 12);
  looping.writeUInt8(12, 13);
  assert.throws(() => decodePacket(looping));
  const truncated = parsonResponse().subarray(0, 60);
  assert.throws(() => decodePacket(truncated));
});

test("browse collects answers from the network and closes its sockets", async () => {
  const sockets = [];
  const createSocket = () => {
    const socket = new EventEmitter();
    socket.sent = [];
    socket.closed = false;
    socket.bind = (_port, callback) => callback();
    socket.addMembership = () => {};
    socket.setMulticastTTL = () => {};
    socket.setMulticastLoopback = () => {};
    socket.send = (message) => {
      socket.sent.push(message);
      socket.emit("message", parsonResponse(), { address: "192.168.1.20" });
      socket.emit("message", Buffer.from("garbage"), { address: "1.2.3.4" });
    };
    socket.close = () => {
      socket.closed = true;
    };
    sockets.push(socket);
    return socket;
  };
  const seen = [];
  const servers = await browse({
    durationMs: 30,
    createSocket,
    onService: (service) => seen.push(service.origin),
  });
  assert.deepEqual(
    servers.map((server) => server.origin),
    ["http://192.168.1.20:1993"],
  );
  assert.ok(seen.length >= 1);
  assert.equal(sockets.length, 2);
  assert.ok(sockets.every((socket) => socket.closed));
});
