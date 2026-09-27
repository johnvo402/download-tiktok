import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, cpSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { checkReleaseTests, latestBranchRun, testDecision } from "../ci-release-gate.mjs";

const sha = "a".repeat(40);
const run = { id: 1, event: "push", head_sha: sha, head_branch: "dev", status: "in_progress", html_url: "https://example.test/ci/1" };
test("only latest default-branch push on the exact SHA is eligible", () => {
  assert.equal(latestBranchRun([run, { ...run, id: 2 }, { ...run, id: 3, head_sha: "b".repeat(40) },
    { ...run, id: 4, head_branch: "v1.0.0" }, { ...run, id: 5, event: "pull_request" }], sha, "dev").id, 2);
  assert.equal(latestBranchRun([run], "b".repeat(40), "dev"), undefined);
});
test("only the successful Windows test job allows reuse", () => {
  assert.equal(testDecision(run, [{ name: "Test Windows x64", conclusion: "success" }]), "reuse");
  assert.equal(testDecision(run, [{ name: "Other tests", conclusion: "success" }]), "wait");
  for (const conclusion of ["failure", "timed_out", "action_required"]) {
    assert.equal(testDecision(run, [{ name: "Test Windows x64", conclusion }]), "fail");
  }
  for (const conclusion of ["cancelled", "skipped"]) {
    assert.equal(testDecision(run, [{ name: "Test Windows x64", conclusion }]), "test");
  }
  assert.equal(testDecision(undefined, []), "test");
  assert.equal(testDecision({ ...run, status: "completed" }, []), "test");
});

function gateFixture(runs, jobs) {
  const outputs = {};
  const github = { rest: { actions: {
    listWorkflowRuns: async args => {
      assert.equal(args.head_sha, sha);
      assert.equal(args.branch, "dev");
      return { data: { workflow_runs: runs } };
    }, listJobsForWorkflowRun: "jobs",
  } }, paginate: async (_, args) => { assert.equal(args.filter, "latest"); return jobs; } };
  const summary = { addRaw() { return this; }, async write() {} };
  return { outputs, github, context: { repo: { owner: "test", repo: "test" }, payload: { repository: { default_branch: "dev" } } },
    core: { setOutput: (key, value) => { outputs[key] = value; }, info() {}, summary }, sleep: async () => {}, attempts: 2 };
}
test("gate falls back on missing, cancelled or still pending CI, and fails closed on errors", async () => {
  const previous = process.env.RELEASE_COMMIT;
  process.env.RELEASE_COMMIT = sha;
  try {
    for (const runs of [[], [run], [{ ...run, status: "completed" }]]) {
      const fixture = gateFixture(runs, []);
      await checkReleaseTests(fixture);
      assert.equal(fixture.outputs["reuse-tests"], "false");
    }
    const good = gateFixture([run], [{ name: "Test Windows x64", conclusion: "success" }]);
    await checkReleaseTests(good);
    assert.equal(good.outputs["reuse-tests"], "true");
    const warming = gateFixture([run], [{ name: "Test Windows x64", conclusion: "success" },
      { name: "Prepare changed release dependencies", status: "in_progress" }]);
    let waits = 0;
    warming.sleep = async () => { waits++; };
    await checkReleaseTests(warming);
    assert.equal(waits, 1);
    assert.equal(warming.outputs["reuse-tests"], "true");
    const bad = gateFixture([run], [{ name: "Test Windows x64", conclusion: "failure" }]);
    await assert.rejects(checkReleaseTests(bad), /Tests failed/);
    assert.deepEqual(bad.outputs, {});
    const brokenApi = gateFixture([], []);
    brokenApi.github.rest.actions.listWorkflowRuns = async () => { throw new Error("API unavailable"); };
    await assert.rejects(checkReleaseTests(brokenApi), /API unavailable/);
    assert.deepEqual(brokenApi.outputs, {});
  } finally {
    if (previous === undefined) delete process.env.RELEASE_COMMIT;
    else process.env.RELEASE_COMMIT = previous;
  }
});

test("tag version updates app manifests without invalidating worker inputs", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "jc-release-version-"));
  try {
    for (const folder of ["scripts", "src-tauri", "python/worker"]) mkdirSync(path.join(root, folder), { recursive: true });
    cpSync(new URL("../set-project-version.mjs", import.meta.url), path.join(root, "scripts/set-project-version.mjs"));
    writeFileSync(path.join(root, "package.json"), JSON.stringify({ version: "0.1.6" }));
    writeFileSync(path.join(root, "package-lock.json"), JSON.stringify({ version: "0.1.6", packages: { "": { version: "0.1.6" } } }));
    writeFileSync(path.join(root, "src-tauri/tauri.conf.json"), JSON.stringify({ version: "0.1.6" }));
    writeFileSync(path.join(root, "src-tauri/Cargo.toml"), '[package]\nname = "ai-video-localization"\nversion = "0.1.6"\n');
    writeFileSync(path.join(root, "src-tauri/Cargo.lock"), '[[package]]\nname = "ai-video-localization"\nversion = "0.1.6"\n');
    writeFileSync(path.join(root, "python/pyproject.toml"), '[project]\nversion = "0.1.6"\n');
    writeFileSync(path.join(root, "python/worker/_version.py"), '__version__ = "0.1.6"\n');
    execFileSync(process.execPath, [path.join(root, "scripts/set-project-version.mjs"), "1.2.3"]);
    for (const file of ["package.json", "package-lock.json", "src-tauri/tauri.conf.json"]) {
      assert.equal(JSON.parse(readFileSync(path.join(root, file))).version, "1.2.3");
    }
    assert.match(readFileSync(path.join(root, "src-tauri/Cargo.toml"), "utf8"), /version = "1.2.3"/);
    assert.match(readFileSync(path.join(root, "src-tauri/Cargo.lock"), "utf8"), /version = "1.2.3"/);
    assert.equal(readFileSync(path.join(root, "python/pyproject.toml"), "utf8"), '[project]\nversion = "0.1.6"\n');
    assert.equal(readFileSync(path.join(root, "python/worker/_version.py"), "utf8"), '__version__ = "0.1.6"\n');
    assert.throws(() => execFileSync(process.execPath, [path.join(root, "scripts/set-project-version.mjs"), "invalid"], { stdio: "pipe" }));
  } finally { rmSync(root, { recursive: true, force: true }); }
});
