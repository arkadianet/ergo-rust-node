# Scala Spec Extraction

## Source: ergo-master (reference implementation)

---

## EIP-37 Difficulty Adjustment

### File: `ergo-core/src/main/scala/org/ergoplatform/mining/difficulty/DifficultyAdjustment.scala`

### Key Constants

```scala
object DifficultyAdjustment {
  val PrecisionConstant: Int = 1000000000  // 10^9 for fixed-point math
}
```

### Activation Logic (HeadersProcessor.scala:338-380)

```scala
val eip37ActivationHeight = 844673  // Mainnet only

// Activation check uses: parentHeight + 1 >= eip37ActivationHeight
// This means: next_height >= 844673
if (settings.networkType.isMainNet && parentHeight + 1 >= eip37ActivationHeight) {
  val epochLength = 128  // EIP-37 epoch length (vs 1024 pre-EIP-37)

  // Epoch boundary check: parentHeight % epochLength == 0
  // This means: (next_height - 1) % 128 == 0
  if (parentHeight % epochLength == 0) {
    // Recalculate difficulty
    difficultyCalculator.eip37Calculate(headers, epochLength)
  } else {
    // Keep parent difficulty
    parent.requiredDifficulty
  }
}
```

**CRITICAL: Activation height comparison:**
- Scala checks: `parentHeight + 1 >= eip37ActivationHeight` (i.e., `next_height >= activation`)
- Epoch boundary: `parentHeight % epochLength == 0` (i.e., `(next_height - 1) % epoch == 0`)

### EIP-37 Calculate Method (lines 80-103)

```scala
def eip37Calculate(previousHeaders: Seq[Header], epochLength: Int): Difficulty = {
  require(previousHeaders.size >= 2, s"at least two headers needed")
  val lastDiff = previousHeaders.last.requiredDifficulty

  // Step 1: Calculate predictive difficulty (linear regression)
  val predictiveDiff = calculate(previousHeaders, epochLength)

  // Step 2: Clamp predictive to 3/2 or 1/2 of last difficulty
  val limitedPredictiveDiff = if (predictiveDiff > lastDiff) {
    predictiveDiff.min(lastDiff * 3 / 2)  // Cap at 150%
  } else {
    predictiveDiff.max(lastDiff / 2)       // Floor at 50%
  }

  // Step 3: Calculate classic (Bitcoin-style) difficulty
  val classicDiff = bitcoinCalculate(previousHeaders, epochLength)

  // Step 4: Average the two
  val avg = (classicDiff + limitedPredictiveDiff) / 2

  // Step 5: Clamp average to 3/2 or 1/2 of last difficulty
  val uncompressedDiff = if (avg > lastDiff) {
    avg.min(lastDiff * 3 / 2)
  } else {
    avg.max(lastDiff / 2)
  }

  // Step 6: Normalize via nBits round-trip
  DifficultySerializer.decodeCompactBits(
    DifficultySerializer.encodeCompactBits(uncompressedDiff)
  )
}
```

### Bitcoin Calculate Method (lines 71-73)

```scala
private def bitcoinCalculate(start: Header, end: Header, epochLength: Int): BigInt = {
  end.requiredDifficulty * desiredInterval.toMillis * epochLength / (end.timestamp - start.timestamp)
}
```

### Interpolate (Linear Regression) Method (lines 131-150)

```scala
// y = a + bx (least squares linear regression)
private[difficulty] def interpolate(data: Seq[(Int, Difficulty)], epochLength: Int): Difficulty = {
  val size = data.size
  if (size == 1) {
    data.head._2
  } else {
    val xy: Iterable[BigInt] = data.map(d => d._1 * d._2)
    val x: Iterable[BigInt] = data.map(d => BigInt(d._1))
    val x2: Iterable[BigInt] = data.map(d => BigInt(d._1) * d._1)
    val y: Iterable[BigInt] = data.map(d => d._2)

    val xySum = xy.sum
    val x2Sum = x2.sum
    val ySum = y.sum
    val xSum = x.sum

    // Fixed-point regression coefficients
    val b: BigInt = (xySum * size - xSum * ySum) * PrecisionConstant / (x2Sum * size - xSum * xSum)
    val a: BigInt = (ySum * PrecisionConstant - b * xSum) / size / PrecisionConstant

    // Extrapolate to next epoch
    val point = data.map(_._1).max + epochLength
    a + b * point / PrecisionConstant
  }
}
```

### Sample Window (previousHeightsRequiredForRecalculation, lines 38-46)

```scala
def previousHeightsRequiredForRecalculation(height: Height, epochLength: Int): Seq[Height] = {
  if ((height - 1) % epochLength == 0 && epochLength > 1) {
    // (0 to useLastEpochs) is INCLUSIVE: 0,1,2,...,8 = 9 points total
    // Returns current epoch boundary + 8 previous = 9 headers
    (0 to useLastEpochs).map(i => (height - 1) - i * epochLength).filter(_ >= 0).reverse
  } else if ((height - 1) % epochLength == 0 && height > epochLength * useLastEpochs) {
    // Same logic, but skip filter (chain is long enough that all heights are positive)
    (0 to useLastEpochs).map(i => (height - 1) - i * epochLength).reverse
  } else {
    // Not at epoch boundary - just need parent
    Seq(height - 1)
  }
}
```

**Sample window:** With `useLastEpochs = 8`, uses `(0 to 8)` which is **9 epoch-boundary headers** (current epoch boundary + 8 previous epoch boundaries). The two branches handle early chain (filter negatives) vs established chain - second branch is an optimization (skip filter when chain is long enough that all heights are guaranteed positive).

---

## Extension Serialization

### File: `ergo-core/src/main/scala/org/ergoplatform/modifiers/history/extension/ExtensionSerializer.scala`

### Serialize Method (lines 12-20)

```scala
override def serialize(obj: Extension, w: Writer): Unit = {
  w.putBytes(idToBytes(obj.headerId))  // 32 bytes (ModifierId)
  w.putUShort(obj.fields.size)          // 2 bytes, big-endian unsigned short
  obj.fields.foreach { case (key, value) =>
    w.putBytes(key)                     // 2 bytes (FieldKeySize)
    w.putUByte(value.length)            // 1 byte, unsigned
    w.putBytes(value)                   // variable length (max 64 bytes)
  }
}
```

### Parse Method (lines 23-36)

```scala
override def parse(r: Reader): Extension = {
  val startPosition = r.position
  val headerId = bytesToId(r.getBytes(Constants.ModifierIdSize))  // 32 bytes
  val fieldsSize = r.getUShort()                                   // 2 bytes big-endian
  val fieldsView = (1 to fieldsSize).toStream.map { _ =>
    val key = r.getBytes(Extension.FieldKeySize)                   // 2 bytes
    val length = r.getUByte()                                      // 1 byte
    val value = r.getBytes(length)                                 // variable
    (key, value)
  }
  val fields = fieldsView.takeWhile(_ => r.position - startPosition < Constants.MaxExtensionSizeMax)
  require(r.position - startPosition < Constants.MaxExtensionSizeMax)
  Extension(headerId, fields, Some(r.position - startPosition))
}
```

**Parse semantics when size limit is hit:**
- `takeWhile` is lazy - it evaluates each field, THEN checks the predicate
- If reading a field causes `bytes_read = r.position - startPosition; bytes_read >= MaxExtensionSizeMax`:
  1. The field bytes are already read (position advanced)
  2. Predicate returns false, field excluded from result
  3. But `bytes_read` is already at or past the limit
  4. `require(...)` fails → **REJECTS the extension**
- Scala does NOT accept partial fields - if declared `fieldsSize` would exceed size limit, parsing fails
- Rust must match: reject if total serialized size >= MaxExtensionSizeMax (32768 bytes)

### Encoding Details

| Field | Encoding | Size |
|-------|----------|------|
| Header ID | Fixed bytes | 32 bytes |
| Field count | Big-endian unsigned short (`putUShort`) | 2 bytes |
| Key | Fixed bytes | 2 bytes |
| Value length | Unsigned byte (`putUByte`) | 1 byte |
| Value | Variable bytes | 0-64 bytes |

**Field ordering:** Insertion order (NOT sorted). Fields are serialized in the order they appear in the `fields` Seq.

### Extension Constants (Extension.scala:44-45)

```scala
val FieldKeySize: Int = 2
val FieldValueMaxSize: Int = 64
```

---

## Extension Digest Computation

### File: `ergo-core/src/main/scala/org/ergoplatform/modifiers/history/extension/Extension.scala`

### kvToLeaf Method (lines 82-83)

```scala
def kvToLeaf(kv: (Array[Byte], Array[Byte])): Array[Byte] =
  Bytes.concat(Array(kv._1.length.toByte), kv._1, kv._2)
  // Format: [key_length: 1 byte][key: 2 bytes][value: variable]
```

### Merkle Tree (lines 88-89)

```scala
def merkleTree(fields: Seq[(Array[Byte], Array[Byte])]): MerkleTree[Digest32] = {
  Algos.merkleTree(LeafData @@ fields.map(kvToLeaf))
}
```

### Digest (ExtensionCandidate.scala:24)

```scala
lazy val digest: Digest32 = Algos.merkleTreeRoot(merkleTree)
```

### Empty Extension Digest (Algos.scala:19, 44-45)

```scala
lazy val emptyMerkleTreeRoot: Digest32 = Algos.hash(LeafData @@ Array[Byte]())
// = Blake2b256(empty_byte_array)
// = 0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8

def merkleTreeRoot(elements: Seq[LeafData]): Digest32 =
  if (elements.isEmpty) emptyMerkleTreeRoot else merkleTree(elements).rootHash
```

**Digest algorithm:** Blake2b-256 Merkle tree. Empty extension uses hash of empty byte array.

---

## Extension Validation Rules

### File: `ergo-core/src/main/scala/org/ergoplatform/nodeView/history/storage/modifierprocessors/ExtensionValidator.scala`

### Validation Rules (lines 20-24)

| Rule ID | Name | Check | Status |
|---------|------|-------|--------|
| 403 | exKeyLength | All keys must be 2 bytes | FATAL (consensus) |
| 404 | exValueLength | All values must be <= 64 bytes | FATAL (consensus) |
| 405 | exDuplicateKeys | No duplicate keys allowed | FATAL (consensus) |
| 406 | exEmpty | Non-genesis blocks must have fields | FATAL (consensus) |
| 401 | exIlEncoding | Interlinks properly packed | FATAL (consensus) |
| 402 | exIlStructure | Interlinks correct structure | FATAL (consensus) |
| 413 | exIlUnableToValidate | Unable to validate interlinks | RECOVERABLE |

All marked FATAL are consensus-critical (block rejection).

---

## DifficultySerializer (nBits encoding)

### File: `ergo-core/src/main/scala/org/ergoplatform/mining/difficulty/DifficultySerializer.scala`

### encodeCompactBits (lines 49-68)

```scala
def encodeCompactBits(requiredDifficulty: BigInt): Long = {
  val value = requiredDifficulty.bigInteger
  var result: Long = 0L
  var size: Int = value.toByteArray.length
  if (size <= 3) {
    result = value.longValue << 8 * (3 - size)
  } else {
    result = value.shiftRight(8 * (size - 3)).longValue
  }
  // Handle sign bit at 0x00800000
  if ((result & 0x00800000L) != 0) {
    result >>= 8
    size += 1
  }
  result |= size << 24
  val a: Int = if (value.signum == -1) 0x00800000 else 0
  result |= a
  result
}
```

### decodeCompactBits (lines 36-44)

```scala
def decodeCompactBits(compact: Long): BigInt = {
  val size: Int = (compact >> 24).toInt & 0xFF
  val bytes: Array[Byte] = new Array[Byte](4 + size)
  bytes(3) = size.toByte
  if (size >= 1) bytes(4) = ((compact >> 16) & 0xFF).toByte
  if (size >= 2) bytes(5) = ((compact >> 8) & 0xFF).toByte
  if (size >= 3) bytes(6) = (compact & 0xFF).toByte
  decodeMPI(bytes)
}
```

---

## Network Parameters

### Mainnet

| Parameter | Value |
|-----------|-------|
| Block interval | 120000 ms (2 minutes) |
| Pre-EIP-37 epoch length | 1024 blocks |
| EIP-37 epoch length | 128 blocks |
| EIP-37 activation height | 844673 |
| useLastEpochs | 8 |
| PrecisionConstant | 1000000000 (10^9) |

---

## Open Questions Resolved

- [x] **Activation height:** Compare `next_height >= activation` (parentHeight + 1 >= 844673)
- [x] **Epoch boundary:** Check `(next_height - 1) % epoch == 0` (parentHeight % epochLength == 0)
- [x] **Sample window:** 8 epoch-boundary headers (useLastEpochs = 8)
- [x] **Extension field ordering:** Insertion order (NOT sorted)
- [x] **Extension digest:** Blake2b-256 Merkle tree
- [x] **Empty extension digest:** Hash of empty byte array = `0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8`
- [x] **Consensus-critical rules:** exKeyLength, exValueLength, exDuplicateKeys, exEmpty, exIlEncoding, exIlStructure
- [x] **putUShort/putUByte endianness:** Big-endian (standard scorex serialization)
