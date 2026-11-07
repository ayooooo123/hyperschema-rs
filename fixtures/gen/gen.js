// Records reference encodings for the hyperschema-rs golden tests.
//
// For every type in a schema and every version it supports, builds a few
// deterministic values, encodes them with the npm reference codecs, decodes the
// bytes again and writes both the decoded value and the hex to
// fixtures/<schema>.json. A case is only recorded when the reference
// re-encodes the decoded value to the same bytes, so every fixture is a value
// the JS codec itself round-trips.
//
// Run with `npm install && npm run gen`.

const fs = require('fs')
const path = require('path')
const Hyperschema = require('hyperschema')

const ROOT = path.join(__dirname, '..')
const SCHEMA_DIR = path.join(ROOT, 'schemas')

// `codec` is the generated module shipped by each package; `source` is what the
// fixture file records. `synthetic` is generated from its schema on the fly.
const MODULES = {
  hyperbee2: {
    package: 'hyperbee2',
    file: 'spec/hyperschema/index.js',
    source: 'hyperbee2@2.21.1'
  },
  autobee: {
    package: 'autobee',
    file: 'encoding/spec/autobee/index.js',
    source: 'autobee@2.12.1'
  },
  'protomux-wakeup': {
    package: 'protomux-wakeup',
    file: 'spec/hyperschema/index.js',
    source: 'protomux-wakeup@2.9.0 (schema reconstructed from the shipped index.js)'
  },
  'hypercore-storage': {
    package: 'hypercore-storage',
    file: 'encoding/spec/hyperschema/index.js',
    source: 'hypercore-storage@3.4.0'
  },
  'hyperdht-address': {
    package: 'hyperdht-address',
    file: 'spec/hyperschema/index.js',
    source: 'hyperdht-address@1.1.1'
  },
  synthetic: { generate: true, source: 'synthetic (features the shipped schemas do not use)' }
}

const VARIANTS = ['full', 'sparse', 'mixed']
const PRIMITIVES = new Set([
  'uint', 'uint1', 'uint2', 'uint3', 'uint4', 'uint5', 'uint6', 'uint7',
  'uint8', 'uint16', 'uint24', 'uint32', 'uint40', 'uint48', 'uint56', 'uint64',
  'int', 'int8', 'int16', 'int24', 'int32', 'int40', 'int48', 'int56', 'int64',
  'float32', 'float64', 'port', 'string', 'utf8', 'ascii', 'hex',
  'biguint64', 'bigint64', 'bool', 'buffer', 'optionalBuffer',
  'fixed8', 'fixed16', 'fixed24', 'fixed32', 'fixed64', 'raw', 'none',
  'ip', 'ipv4', 'ipv6', 'ipAddress', 'ipv4Address', 'ipv6Address'
])
const BITWISE = new Set(['uint1', 'uint2', 'uint3', 'uint4', 'uint5', 'uint6', 'uint7'])
const UINT_BITS = {
  uint8: 8, uint16: 16, uint24: 24, uint32: 32, uint40: 40, uint48: 48, uint56: 56, uint64: 53
}
const INT_BITS = {
  int8: 8, int16: 16, int24: 24, int32: 32, int40: 40, int48: 48, int56: 56, int64: 52
}

function fqnOf(t) {
  return t.namespace ? '@' + t.namespace + '/' + t.name : t.name
}

function makeRng(seed) {
  let s = seed >>> 0
  return () => {
    s = (s + 0x6d2b79f5) >>> 0
    let t = s
    t = Math.imul(t ^ (t >>> 15), t | 1)
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61)
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

function loadCodec(name, schema) {
  const entry = MODULES[name]
  if (entry.file) {
    // The packages hide their spec files behind `exports`, so resolve by path.
    const root = path.dirname(require.resolve(entry.package))
    return require(path.join(root, entry.file))
  }
  // One file per schema: `require` caches by path, so a shared name would hand
  // the second generated schema the first one's codec.
  const file = path.join(__dirname, `.generated-${name}.js`)
  const code = new Hyperschema(schema).toCode({ filename: file })
  fs.writeFileSync(file, code)
  return require(file)
}

function kindOf(t) {
  if (t.alias) return 'alias'
  if (t.enum) return 'enum'
  if (t.array) return 'array'
  if (t.record) return 'record'
  if (t.external) return 'external'
  if (t.versions) return 'versioned'
  return 'struct'
}

function main() {
  for (const [name, entry] of Object.entries(MODULES)) {
    const dir = path.join(SCHEMA_DIR, name)
    const schema = JSON.parse(fs.readFileSync(path.join(dir, 'schema.json'), 'utf8'))
    const codec = loadCodec(name, schema)
    const types = new Map(schema.schema.map((t) => [fqnOf(t), t]))
    const targets = versionedTargets(schema)
    const cases = []
    let skipped = 0

    for (const t of schema.schema) {
      const fqn = fqnOf(t)
      const kind = kindOf(t)

      if (kind === 'alias' || kind === 'external') continue

      if (kind === 'versioned') {
        for (const def of t.versions) {
          const inner = types.get(def.type)
          // Legacy versions are decode-only in JS: their `preencode` throws, and
          // mapped versions return a shape the raw schema type does not have.
          if (!inner || kindOf(inner) === 'external' || def.map) {
            console.log(`  skip ${fqn} v${def.version}: ${!inner ? 'unknown type' : kindOf(inner) === 'external' ? 'decode-only external' : 'JS maps the value'}`)
            skipped++
            continue
          }
          if (touchesExternal(inner, types, new Set())) {
            console.log(`  skip ${fqn} v${def.version}: payload reaches an external type`)
            skipped++
            continue
          }
          const ctx = { variant: 'full', rand: makeRng(hash(fqn) + def.version), version: schema.version }
          const value = Object.assign({ version: def.version }, sampleType(def.type, ctx, types))
          const rec = encodeCase(codec, fqn, value, schema.version)
          if (!rec) {
            console.log(`  skip ${fqn} v${def.version}: the reference does not round-trip the sample`)
            skipped++
            continue
          }
          cases.push({
            name: fqn,
            version: schema.version,
            hex: rec.hex,
            value: Object.assign(
              { version: def.version },
              jsonSafe(toFixture(def.type, rec.decoded, types))
            )
          })
        }
        continue
      }

      if (targets.has(fqn)) {
        // The versioned wrapper carries the version prefix; cases above cover it.
        skipped++
        continue
      }

      if (kind === 'enum' || kind === 'array' || kind === 'record') {
        for (const variant of VARIANTS) {
          const ctx = { variant, rand: makeRng(hash(fqn)), version: schema.version }
          const value = sampleType(fqn, ctx, types)
          const rec = encodeCase(codec, fqn, value, schema.version)
          if (!rec) {
            console.log(`  skip ${fqn} ${variant}: the reference does not round-trip the sample`)
            skipped++
            continue
          }
          cases.push({
            name: fqn,
            version: schema.version,
            hex: rec.hex,
            value: jsonSafe(toFixture(fqn, rec.decoded, types))
          })
        }
        continue
      }

      for (let version = 1; version <= schema.version; version++) {
        const variants = version === schema.version ? VARIANTS : ['full', 'sparse']
        for (const variant of variants) {
          const ctx = { variant, rand: makeRng(hash(fqn) + version * 31), version }
          const value = sampleStruct(t, ctx, types)
          const rec = encodeCase(codec, fqn, value, version)
          if (!rec) {
            console.log(`  skip ${fqn} v${version} ${variant}: the reference does not round-trip the sample`)
            skipped++
            continue
          }
          cases.push({
            name: fqn,
            version,
            hex: rec.hex,
            value: jsonSafe(toFixture(fqn, rec.decoded, types))
          })
        }
      }
    }

    // Frames written by an older codec are still readable, so they are recorded
    // next to the current encodings.
    if (name === 'hyperbee2') cases.push(...legacyBlockFrames(codec, types))
    if (name === 'synthetic') cases.push(...frameErrors(codec, schema.version))

    const out = {
      source: entry.source,
      version: schema.version,
      cases: cases.map((c) => ({
        name: c.name,
        version: c.version,
        hex: c.hex,
        value: c.value,
        decodeVersion: c.decodeVersion,
        reencode: c.reencode,
        error: c.error,
        note: c.note
      })),
      skipped
    }
    const file = path.join(ROOT, name + '.json')
    fs.writeFileSync(file, JSON.stringify(out, null, 1) + '\n')
    console.log(name, cases.length, 'cases,', skipped, 'skipped')
  }
}

// hyperbee2 schema version 1 had no `metadata.mappings`, so its metadata frame
// carries no flags word. Those blocks are still on disk, so the current codec
// has to read them like JS does: inside the frame.
function legacyBlockFrames(codec, types) {
  const schema = JSON.parse(
    fs.readFileSync(path.join(SCHEMA_DIR, 'hyperbee2', 'schema.json'), 'utf8')
  )
  const legacy = JSON.parse(JSON.stringify(schema))
  legacy.version = 1
  for (const t of legacy.schema) {
    if (t.namespace === 'bee' && t.name === 'metadata') {
      t.fields = t.fields.filter((f) => (f.version || 1) <= 1)
      t.flagsPosition = -1
    }
  }
  const file = path.join(__dirname, '.legacy-hyperbee2.js')
  fs.writeFileSync(file, new Hyperschema(legacy).toCode({ filename: file }))
  const old = require(file)
  fs.unlinkSync(file)

  const value = {
    type: 1,
    seq: 0,
    checkpoint: 5,
    batch: { start: 3, end: 9 },
    previous: null,
    metadata: { cores: [{ key: Buffer.alloc(32, 0x44), fork: 0, length: 7, treeHash: null }] },
    // A field after the metadata frame: its first byte must not be read as the
    // frame's flags word.
    tree: [{ keys: [], children: [] }],
    keys: null,
    values: null,
    cohorts: null,
    t: null,
    invalidation: null
  }
  const hex = old.encode('@bee/block-1', value, 1).toString('hex')
  const decoded = codec.decode('@bee/block-1', Buffer.from(hex, 'hex'), 2)
  return [
    {
      name: '@bee/block-1',
      version: 1,
      decodeVersion: 2,
      hex,
      reencode: codec.encode('@bee/block-1', decoded, 1).toString('hex'),
      value: jsonSafe(toFixture('@bee/block-1', decoded, types)),
      note: 'schema v1 metadata frame without a flags word, decoded at version 2'
    }
  ]
}

// A frame length is a promise. One that reaches past the buffer, or one shorter
// than the value inside, must fail instead of reading the next field.
function frameErrors(codec, version) {
  const value = {
    id: 1,
    one: { id: 2, label: null },
    more: [],
    vb: { version: 1, id: 3, label: null },
    tail: null
  }
  // id | one(len=2, inner) | more | vb(version=1, len=2, inner) | flags
  const valid = '01' + '02' + '0002' + '00' + '01' + '02' + '0003' + '00'
  const hex = codec.encode('@syn/frame-holder', value, version).toString('hex')
  if (hex !== valid) throw new Error(`unexpected @syn/frame-holder layout: ${hex}`)

  const cases = [
    ['frame reaching past the buffer', '01' + '09' + '0002' + '00' + '01' + '02' + '0003' + '00'],
    ['frame shorter than the value inside', '01' + '00' + '0002' + '00' + '01' + '02' + '0003' + '00'],
    ['versioned body frame reaching past the buffer', '01' + '02' + '0002' + '00' + '01' + '7f' + '0003' + '00']
  ]
  for (const [note, bytes] of cases) {
    let threw = false
    try {
      codec.decode('@syn/frame-holder', Buffer.from(bytes, 'hex'), version)
    } catch {
      threw = true
    }
    if (!threw) throw new Error(`the reference accepts a broken frame: ${note}`)
  }
  return cases.map(([note, bytes]) => ({
    name: '@syn/frame-holder',
    version,
    hex: bytes,
    error: true,
    note
  }))
}

// Types used as a versioned payload get their version byte from the wrapper, so
// they are only exercised through it.
function versionedTargets(schema) {
  const out = new Set()
  for (const t of schema.schema) {
    if (!t.versions) continue
    for (const d of t.versions) out.add(d.type)
  }
  return out
}

function touchesExternal(t, types, seen) {
  if (t.array) return touchesExternal(types.get(t.type) || generic(t.type), types, seen)
  if (t.record) return false
  const kind = kindOf(t)
  if (kind === 'external') return true
  if (kind !== 'struct') return false
  const fqn = fqnOf(t)
  if (seen.has(fqn)) return false
  seen.add(fqn)
  for (const f of t.fields) {
    if (f.constant !== undefined) continue
    if (generic(f.type) || !types.has(f.type)) continue
    if (touchesExternal(types.get(f.type), types, seen)) return true
  }
  return false
}

function generic(name) {
  return PRIMITIVES.has(name) ? { name, namespace: null } : null
}

// Encode, decode and re-encode. Records have to be compared as objects, so the
// round trip check runs before the value is turned into its fixture shape.
function encodeCase(codec, fqn, value, version) {
  let bytes
  let decoded
  try {
    bytes = codec.encode(fqn, value, version)
    decoded = codec.decode(fqn, bytes, version)
  } catch (err) {
    return null
  }
  const again = codec.encode(fqn, decoded, version)
  // Only values the reference itself round-trips are stable enough to assert.
  if (!again.equals(bytes)) return null
  return { hex: bytes.toString('hex'), decoded }
}

// Records decode to objects in JS but are ordered pairs on the wire, so the
// fixture value spells them out as pairs in wire order.
function toFixture(name, value, types) {
  if (value === null || value === undefined) return null
  if (PRIMITIVES.has(name)) return value
  const def = types.get(name)
  if (!def) return value
  switch (kindOf(def)) {
    case 'alias':
      return toFixture(def.alias, value, types)
    case 'record': {
      const out = []
      for (const key of Object.keys(value)) out.push([key, toFixture(def.value, value[key], types)])
      return out
    }
    case 'array':
      return value.map((item) => toFixture(def.type, item, types))
    case 'struct': {
      const out = {}
      for (const f of def.fields) {
        const v = value[f.name]
        if (v === null || v === undefined) {
          out[f.name] = null
        } else if (f.array) {
          out[f.name] = v.map((item) => toFixture(f.type, item, types))
        } else {
          out[f.name] = toFixture(f.type, v, types)
        }
      }
      return out
    }
    default:
      return value
  }
}

// Buffers become hex, BigInt becomes Number and null-prototype records become
// plain objects so the case value can be compared field by field in Rust.
function jsonSafe(value) {
  return JSON.parse(
    JSON.stringify(value, (key, v) => {
      if (typeof v === 'bigint') return Number(v)
      if (v && typeof v === 'object' && v.type === 'Buffer' && Array.isArray(v.data)) {
        return Buffer.from(v.data).toString('hex')
      }
      return v
    })
  )
}

// ---------------------------------------------------------------------------
// value sampling

function presentOptional(index, ctx) {
  if (ctx.variant === 'sparse') return false
  if (ctx.variant === 'full') return true
  return index % 2 === 0
}

function sampleStruct(def, ctx, types) {
  const out = {}
  ctx.depth = (ctx.depth || 0) + 1
  def.fields.forEach((f, i) => {
    if (f.constant !== undefined) {
      out[f.name] = f.constant
      return
    }
    const required = !!f.required || (!f.array && (f.type === 'bool' || BITWISE.has(f.type)))
    const visible = required || (f.version || 1) <= ctx.version
    if (!visible) {
      out[f.name] = null
      return
    }
    if (!required && (ctx.depth > 2 || !presentOptional(i, ctx))) {
      out[f.name] = null
      return
    }
    out[f.name] = f.array ? sampleArray(f.type, ctx, types) : sampleType(f.type, ctx, types)
  })
  ctx.depth--
  return out
}

function sampleArray(name, ctx, types) {
  const len = ctx.depth > 2 ? 0 : ctx.variant === 'sparse' ? 0 : ctx.variant === 'mixed' ? 1 : 3
  const out = []
  for (let i = 0; i < len; i++) out.push(sampleType(name, ctx, types))
  return out
}

function sampleType(name, ctx, types) {
  if (PRIMITIVES.has(name)) return samplePrim(name, ctx)
  const def = types.get(name)
  if (!def) throw new Error('cannot sample unknown type ' + name)
  switch (kindOf(def)) {
    case 'alias':
      return sampleType(def.alias, ctx, types)
    case 'enum': {
      const i = ctx.variant === 'full' ? def.enum.length - 1 : Math.floor(ctx.rand() * def.enum.length)
      const key = typeof def.enum[i] === 'string' ? def.enum[i] : def.enum[i].key
      return def.strings ? key : i + (def.offset === undefined ? 1 : def.offset)
    }
    case 'array':
      return sampleArray(def.type, ctx, types)
    case 'record': {
      const out = {}
      const len = ctx.variant === 'sparse' ? 0 : ctx.variant === 'mixed' ? 1 : 3
      for (let i = 0; i < len; i++) out['k' + i] = sampleType(def.value, ctx, types)
      return out
    }
    case 'struct':
      return sampleStruct(def, ctx, types)
    case 'versioned': {
      const inner = def.versions[def.versions.length - 1]
      return Object.assign({ version: inner.version }, sampleType(inner.type, ctx, types))
    }
    case 'external':
      return null
    default:
      throw new Error('cannot sample ' + name)
  }
}

function samplePrim(name, ctx) {
  const r = ctx.rand
  const extreme = ctx.variant === 'full'
  switch (name) {
    case 'uint':
      return extreme ? 0xffffffff + 1 : pick([0, 1, 0xfc, 0xfd, 0x1000, 0x123456], r)
    case 'uint8':
      return extreme ? 0xff : randUint(8, r)
    case 'uint16':
      return extreme ? 0xffff : randUint(16, r)
    case 'uint24':
      return extreme ? 0xffffff : randUint(24, r)
    case 'uint32':
      return extreme ? 0xffffffff : randUint(32, r)
    case 'uint40':
      return extreme ? 2 ** 40 - 1 : randUint(40, r)
    case 'uint48':
      return extreme ? 2 ** 48 - 1 : randUint(48, r)
    case 'uint56':
      return extreme ? 2 ** 53 - 1 : randUint(53, r)
    case 'uint64':
      return extreme ? 2 ** 53 - 1 : randUint(53, r)
    case 'int':
      return extreme ? -(2 ** 52 - 1) : r() < 0.5 ? -Math.floor(r() * 1e6) : Math.floor(r() * 1e6)
    case 'int8':
      return extreme ? -128 : randInt(8, r)
    case 'int16':
      return extreme ? -32768 : randInt(16, r)
    case 'int24':
      return extreme ? -(2 ** 23) : randInt(24, r)
    case 'int32':
      return extreme ? -(2 ** 31) : randInt(32, r)
    case 'int40':
      return extreme ? -(2 ** 39) : randInt(40, r)
    case 'int48':
      return extreme ? -(2 ** 47) : randInt(48, r)
    case 'int56':
      return extreme ? -(2 ** 52 - 1) : randInt(52, r)
    case 'int64':
      return extreme ? -(2 ** 52 - 1) : randInt(52, r)
    case 'float32':
      return extreme ? -1.5 : pick([0.5, 1.25, -2.75, 1024.5], r)
    case 'float64':
      return extreme ? 1e18 : pick([0.5, -1.25, 1 / 3, 2 ** 40 + 0.25], r)
    case 'port':
      return extreme ? 65535 : 3000 + Math.floor(r() * 40000)
    case 'string':
    case 'utf8':
      return extreme ? 'héllo wörld ✓' : pick(['', 'a', 'pear', 'a longer value with spaces'], r)
    case 'ascii':
      return pick(['', 'abc', 'ASCII only'], r)
    case 'hex':
      return 'deadbeef'
    case 'buffer':
      return extreme ? Buffer.alloc(40, 0xab) : Buffer.from([1, 2, 3])
    case 'optionalBuffer':
      return r() < 0.25 ? null : extreme ? Buffer.alloc(9, 7) : Buffer.from([9, 8])
    case 'fixed8':
      return Buffer.alloc(8, 0x11)
    case 'fixed16':
      return Buffer.alloc(16, 0x22)
    case 'fixed24':
      return Buffer.alloc(24, 0x33)
    case 'fixed32':
      return Buffer.alloc(32, 0x44)
    case 'fixed64':
      return Buffer.alloc(64, 0x55)
    case 'raw':
      return Buffer.from('raw tail')
    case 'none':
      return null
    case 'biguint64':
      return BigInt(extreme ? 2 ** 53 - 1 : 42)
    case 'bigint64':
      return BigInt(extreme ? -(2 ** 52) : -7)
    case 'bool':
      return ctx.variant === 'sparse' ? false : r() < 0.5
    case 'ip':
      return extreme ? '2001:db8::1' : '10.1.2.3'
    case 'ipv4':
      return '10.1.2.3'
    case 'ipv6':
      return '2001:db8::1'
    case 'ipAddress':
      return { host: extreme ? '2001:db8::2' : '10.1.2.4', family: extreme ? 6 : 4, port: 4242 }
    case 'ipv4Address':
      return { host: '10.1.2.5', family: 4, port: 4243 }
    case 'ipv6Address':
      return { host: '2001:db8::3', family: 6, port: 4244 }
    default: {
      if (BITWISE.has(name)) {
        const bits = Number(name.slice(4))
        return extreme ? 2 ** bits - 1 : Math.floor(r() * 2 ** bits)
      }
      throw new Error('cannot sample primitive ' + name)
    }
  }
}

function pick(values, r) {
  return values[Math.floor(r() * values.length)]
}

function randUint(bits, r) {
  return Math.floor(r() * Math.min(2 ** bits, 2 ** 53 - 1))
}

function randInt(bits, r) {
  const max = 2 ** (bits - 1) - 1
  return Math.floor(r() * 2 * max) - max
}

function hash(text) {
  let h = 2166136261
  for (let i = 0; i < text.length; i++) {
    h = Math.imul(h ^ text.charCodeAt(i), 16777619)
  }
  return h >>> 0
}

main()
