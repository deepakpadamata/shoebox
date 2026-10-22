# Using your own relay

Devices try to connect directly. When they cannot (different networks, strict firewalls) they fall back to a
**relay**: a small server that forwards already-encrypted traffic between them. By default Shoebox uses the
public relays that the iroh project operates. A relay sees which device keys talk to each other, when and how
much; it cannot read photos or metadata.

## Why run your own

* you do not want any third party to see connection metadata;
* you want predictable performance for a family spread across continents;
* you are on a network that blocks the public relays.

## Run a relay

Shoebox uses the standard iroh relay protocol, so you run iroh's relay server (`iroh-relay`, from the
[iroh](https://github.com/n0-computer/iroh) project) on any machine with a public name and a TLS certificate.
Follow the iroh relay documentation for the current install and configuration steps; you end up with a URL such as
`https://relay.example.org`.

> The instructions above were not exercised as part of this project's own tests (the automated tests use
> direct loopback connections and no relay), so check the relay's own documentation and test a connection
> before relying on it.

## Tell the devices

* **Desktop and phone:** *Settings → Connection → Relay* → *Custom* → enter the URL. Choose *Off* to allow only
  direct connections (devices on the same network, or reachable addresses).
* **Keeper:** `shoebox-keeper run --relay https://relay.example.org`, or `--relay off`.

Every device in a library must use a relay they can all reach. Use the same URL on all of them.

## Check that it works

*Devices* shows how each device is reached: **Same network**, **Direct**, or **Via relay**, with the latency.
If two devices on different networks show *Connecting* for a long time, they cannot reach a common relay.
