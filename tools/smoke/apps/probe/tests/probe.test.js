test("a probe's last outcome is kept", () => {
  call("record", { probe: "network", reached: true, detail: "200" });
  call("record", { probe: "network", reached: false, detail: "blocked" });
  assert.deepEqual(call("results", {}), [{ probe: "network", reached: false, detail: "blocked" }]);
});
