// Driver del arnés diferencial: lee un fixture JSON (`{files, queries}`) de argv[2], lo pasa por las
// funciones VERBATIM del prototipo y emite la salida normalizada a stdout. Lo invoca `tests/differential.rs`.
import { readFileSync } from "fs";
import { analyzeFixture } from "./proto.mjs";

const path = process.argv[2];
if (!path) {
  process.stderr.write("uso: node run.mjs <fixture.json>\n");
  process.exit(2);
}
const input = JSON.parse(readFileSync(path, "utf8"));
process.stdout.write(JSON.stringify(analyzeFixture(input)));
