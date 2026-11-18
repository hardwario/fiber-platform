# The Bluetooth profile

How HARDWARIO Manager on a phone talks to a FIBER over Bluetooth Low Energy: to see which FIBER
it is, how far its setup is, and to put it on Wi-Fi. Nothing else goes over Bluetooth; everything
else is MQTT ([`mqtt-api.md`](mqtt-api.md)).

**Status: a draft (T28), for the maintainer and then for the people who make HARDWARIO Manager.**
Nothing is built on it until they agree (D3). Not tried with a phone.

## What it is for

A FIBER that has no network yet: written by FIBER Setup without Wi-Fi, or moved to another
place. Someone presses a button on the FIBER, opens HARDWARIO Manager, reads the code on the
FIBER's display into the phone, and picks a Wi-Fi network. From then on the FIBER is on the
network, and everything else is MQTT and SSH.

## The service

One primary service, with three characteristics. The UUIDs are the FIBER's own:

| | UUID | Properties | Needs pairing |
|---|---|---|---|
| The service | `ea5e3690-d1ce-481c-91c3-6bb8d435935a` | | |
| `info` | `ea5e3691-d1ce-481c-91c3-6bb8d435935a` | read | no |
| `rpc_rx` | `ea5e3692-d1ce-481c-91c3-6bb8d435935a` | write | yes |
| `rpc_tx` | `ea5e3693-d1ce-481c-91c3-6bb8d435935a` | notify | yes |

**`info`** is a UTF-8 JSON object that says which FIBER it is, without pairing, so the app can
show it in its list:

```json
{"schema":1,"name":"fiber-abcdef12","hardware":"FIBER-R2 2.1","version":"0.2.0"}
```

`schema` is the version of this profile, now **1**, under the same rule as MQTT's: it changes
with any change an app would notice. An app written for another number says so and stops.

**`rpc_rx` and `rpc_tx`** carry requests and their answers, the same JSON as on MQTT's `cmd`
and `rsp/{id}`:

```json
{"id":"a1","op":"wifi.scan"}
```
```json
{"id":"a1","ok":true,"data":{"networks":[{"ssid":"Office","signal":-52,"security":"wpa2"}]}}
```

## Messages in pieces

A request or an answer can be longer than one write or notification (an ATT MTU of 23 bytes
leaves 20). Each message is UTF-8 JSON, at most 4 KB, sent in pieces:

- **Each piece** starts with one byte: bit 7 set on the last piece of a message, bit 6 set on
  the first, bits 0 to 5 the number of the piece from 0, counting on from 63 to 0 (a message of
  4 KB at the smallest MTU takes 205 pieces).
- **The rest of the piece** is the next part of the message: at most the MTU less 3, less this
  byte. A message that fits in one piece has bits 7 and 6 both set.
- **A first piece** starts a new message; whatever came before and did not end is dropped. So is
  a message longer than 4 KB, and one with a piece missing (its number is not the next).

Both sides cut and join the same way. The FIBER answers on `rpc_tx` only when the app has
turned on its notifications.

## The requests

Only these five. Each gets one answer; the times are the longest the app waits.

| `op` | `params` | `data` | Answer within |
|---|---|---|---|
| `device.info` | | `{"name","hardware","serial","version"}` | 1 s |
| `setup.state` | | `{"state","step","steps","component"}`: `state` as `fiber platform status` has it (`running`, `done`, `partly`, `failed`, or `none` before the first apply) | 1 s |
| `wifi.scan` | | `{"networks":[{"ssid","signal","security"}]}`: the strongest first, `signal` in dBm, `security` one of `open`, `wpa2`, `wpa3` | 15 s |
| `wifi.connect` | `{"ssid","password"}` | `{"address"}`, the IPv4 address it got | 45 s |
| `net.status` | | `{"ethernet":{"up","address"},"wifi":{"ssid","signal","address"}}`, `null` for what is not there | 1 s |

Errors of their own: `wifi.connect` answers `wrong password`, `no such network` or `no address`
(it joined, and no DHCP server answered). The password is not kept anywhere but in
NetworkManager's own connection, readable by root alone.

## Who may talk

- **Nothing without a press on the FIBER.** The FIBER advertises, and takes a pairing, only for 5
  minutes after *Set up from the phone* in its menu (T21). The display says so, and shows the
  time left.
- **Pairing with a passkey:** LE Secure Connections, the FIBER shows the six digits on its display
  and the phone types them in. A link that is not paired that way, or is only encrypted, cannot
  write `rpc_rx` or subscribe to `rpc_tx`; `info` stays readable.
- **One phone at a time.** A second connection is refused while one is open.
- **The advertisement** carries the service's UUID and the FIBER's name (`fiber-abcdef12`), so
  the app finds FIBERs without connecting.
- **After the 5 minutes,** or when the menu closes it, advertising stops and an open link is
  dropped. Bonds are not kept: the next time is paired again.

## If phones pair badly

Bluetooth pairing with a passkey works differently on different phones. Should the bench show it
unreliable, the plan is to keep the same six digits and the same flow on the screen, and do a key
exchange of our own above an unpaired link: a password-authenticated one, with the six digits as
the password, and the requests encrypted with the key it gives. That is a change of `schema`,
and is not designed here.

## Not decided here

- Whether *Set up from the phone* is also offered without a press when a FIBER has no network at
  all after its first start.
- What the app shows when `setup.state` is `partly` or `failed`.
