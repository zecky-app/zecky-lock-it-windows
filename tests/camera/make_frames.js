// Moving QR codes made the way Keystone (fountain parts, upper case) and
// Cupcake (sequenced, lower case, only parts 1..n) make them.
// node make_frames.js <folder> <bytes> <keystone|cupcake> <frames>
const { UR, UREncoder } = require('@ngraveio/bc-ur');
const QR = require('qrcode');
const fs = require('fs');
const crypto = require('crypto');
const [folder, bytes, style, frames] = [process.argv[2], +process.argv[3], process.argv[4], +process.argv[5]];
const body = crypto.randomBytes(bytes);
const cbor = Buffer.concat([Buffer.from([0xa1, 0x01, 0x59, bytes >> 8, bytes & 255]), body]);
const encoder = new UREncoder(new UR(cbor, 'zcash-pczt'), 200, 0);
const count = style === 'cupcake' ? encoder.fragmentsLength : frames;
const texts = [];
for (let i = 0; i < count; i++) {
  const part = encoder.nextPart();
  texts.push(style === 'cupcake' ? part.toLowerCase() : part.toUpperCase());
}
(async () => {
  fs.mkdirSync(folder, { recursive: true });
  for (let i = 0; i < texts.length; i++) {
    await QR.toFile(`${folder}/f${String(i).padStart(4, '0')}.png`, texts[i], { errorCorrectionLevel: 'L', margin: 2, width: 520 });
  }
  fs.writeFileSync(`${folder}/texts.json`, JSON.stringify(texts));
  console.log(JSON.stringify({ fragments: encoder.fragmentsLength, frames: texts.length }));
})();
