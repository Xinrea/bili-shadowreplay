import { readFile } from "node:fs/promises";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/recording-storage.ts", import.meta.url), "utf8");
const module = new vm.SourceTextModule(ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
}).outputText);
await module.link(() => { throw new Error("Unexpected storage helper dependency"); });
await module.evaluate();
export const recordingStorage = module.namespace;
