**Title:** `bootstrap/network` gives no way to enable delete protection on the ingress load
balancer, and turning it on outside Terraform becomes permanent plan drift

**Type:** enhancement with an operational edge. Nothing breaks today; the gap shows up the first
time an operator protects the load balancer by hand.

## Where

`shared-modules/bootstrap/network/loadbalancer.tf`:

```hcl
resource "hcloud_load_balancer" "nginx" {
  count              = var.load_balancer_enabled == true ? 1 : 0
  name               = var.load_balancer_name
  load_balancer_type = var.load_balancer_type
  location           = var.hcloud_location
  depends_on         = [hcloud_network.backend]
}
```

`delete_protection` is not set, and the module has no variable for it.

## Why it matters

Once ingress-nginx is installed, this load balancer is the cluster's public entrypoint: its IPv4
and IPv6 addresses are what DNS points at. Deleting it, by hand, by a mistaken `terraform destroy`
of the wrong root, or by the Hetzner cloud controller when the ingress Service is removed, loses
those addresses. The DNS records then point at nothing until new addresses are propagated.
Hetzner's delete protection is the guard against exactly that.

The module offers no way to set it. An operator who enables it in the Console or through the API
gets permanent drift instead. The hcloud provider (1.66.0) treats an unset `delete_protection` as
`false`. So every later plan of the bootstrap stage proposes
`delete_protection: true -> false` on `module.bootstrap_network.hcloud_load_balancer.nginx[0]`,
and any apply of that stage, however unrelated, silently removes the protection.

## Suggested fix

Add a variable and pass it through:

```hcl
# variables.tf
variable "load_balancer_delete_protection" {
  description = "Enable Hetzner delete protection on the ingress load balancer"
  type        = bool
  default     = false
}

# loadbalancer.tf
resource "hcloud_load_balancer" "nginx" {
  # ...existing arguments...
  delete_protection = var.load_balancer_delete_protection
}
```

Defaulting to `false` keeps existing roots unchanged. A root that sets it to `true` plans a single
in-place update and no replacement. A root whose operator already enabled protection by hand plans
clean again.

The same reasoning applies to the other long-lived network resources the module creates, such as
the floating IP the VPN router uses. That can be a follow-up.

## Downstream state (FAU)

As of 23 September 2026, FAU's `lb-fau` has delete protection turned on by hand in the Hetzner
Console. Until the variable exists, FAU's stage 1 plan shows the one expected change above. **That
change must not be applied.** When the variable lands, FAU sets
`load_balancer_delete_protection = true` in `infrastructure/1-bootstrap` and the drift disappears.
