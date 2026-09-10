---
name: fau-vpn
description: Use when reaching the FAU k3s cluster or anything else on the private 10.0.0.0/14 network from the agent container - kubectl, ssh to a node, curl against an internal address - and especially when such a command hangs or times out, because the WireGuard tunnel does not survive a container recreation and nothing raises it at start.
---

# Reaching FAU's private network from the agent container

`kubeconfig.yaml` points at `https://10.0.1.250:6443`. That address only exists behind the
`fau` WireGuard tunnel, or through the vpn-router as an SSH bastion. The tunnel is
**per-container**: nothing raises it at start, and any container recreation drops it. Raising it
again is the normal first step of a session that touches the cluster, not a sign of breakage.

## Bring the tunnel up

Check before raising - `wg-quick up` on an existing interface just fails with
``wg-quick: `fau' already exists``, so the guard keeps the output clean.

```bash
sudo wg show fau 2>/dev/null || sudo wg-quick up /infra-runtime/infrastructure/.config/fau.conf
```

Expect `ip link add fau`, address `10.0.200.2/32`, MTU 1420 and a route for `10.0.0.0/14`.

## Verify before trusting it

Both checks, in this order. A tunnel that is up but never handshakes looks identical to a working
one until `kubectl` hangs.

```bash
sudo wg show fau                     # want: "latest handshake" a few seconds ago
export KUBECONFIG=/infra-runtime/infrastructure/.config/kubeconfig.yaml
timeout 30 kubectl get nodes         # want: master, vpn-router, cworker-1 all Ready
```

Always wrap the first `kubectl` in `timeout` - without the tunnel it hangs on a private address
rather than failing, and a hung foreground command stalls the session.

`kube-system` pods sitting `Pending` are expected until stage 2 installs the Hetzner CCM, which
clears the `node.cloudprovider.kubernetes.io/uninitialized` taint. That is not a tunnel fault.

## When it does not come up

- **`Cannot find device` / `Operation not permitted`** - the capabilities are gone. Check
  `ls /dev/net/tun` and `grep CapBnd /proc/self/status` (NET_ADMIN is bit 12, `0x1000`), and
  `sudo -n -l` for the `wg-quick`/`wg` rule. If they are missing the container was recreated
  without `devices: /dev/net/tun` and `cap_add: NET_ADMIN`, both of which are in
  `docker-compose.yml`. **Report it and stop.** Rebuilding or recreating the agent service is a
  host action; doing it from inside kills every session in the box.
- **Interface up, no handshake** - the peer endpoint in `fau.conf` is the vpn-router's public
  address on UDP 51890. Check the Hetzner Cloud console for a changed public IP and the firewall
  for that UDP port. Do not print the config file; it holds a private key.
- **Handshake fine, `kubectl` still times out** - confirm the route with `ip route get 10.0.1.250`;
  it should leave via `fau`.

## Fallback: the SSH bastion

The bastion needs no capabilities and works whenever the tunnel does not. Jump through the
vpn-router on port 54322 with the admin key at
`/infra-runtime/infrastructure/.config/ssh_key/id_ed25519`, using `ProxyCommand` to reach the
master and running `k3s kubectl` there. Always pass `-o BatchMode=yes -o ConnectTimeout=10` so a
missing key fails instead of hanging.

An `ssh -L 6443:10.0.1.250:6443` forward for local `kubectl` should work - k3s normally puts
`127.0.0.1` in the API server certificate SANs - but that has not been tested.

## Keep out of the transcript and /workspace

`fau.conf`, the kubeconfig, the node token and the SSH keys all live under
`/infra-runtime/infrastructure/.config` and stay there. Never `cat` them, never copy them into
`/workspace`. Redact `PrivateKey`, `PublicKey` and `PresharedKey` if surrounding context must be
shown.
