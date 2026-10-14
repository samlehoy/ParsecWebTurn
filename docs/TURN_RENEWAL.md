# TURN channel renewal

The native client refreshes a ready TURN channel binding on the first outbound
send after 240 seconds since its last successful bind. This uses the existing
send-triggered refresh path; no additional periodic timer or setting is added.
The same constant applies to normal and diagnostic builds.

The allocation timer remains half the initial server-provided lifetime, and the
permission timer remains 120 seconds. Authentication, retries, readiness, route
selection, SCTP and media processing are unchanged.

[RFC 8656 sections 9 and 12](https://datatracker.ietf.org/doc/html/rfc8656)
specify five-minute permission lifetimes and ten-minute channel-binding
lifetimes. ChannelBind also refreshes the corresponding permission. Refreshing
after four minutes leaves a margin before the permission boundary, provided
outbound traffic continues; sending media alone does not renew a permission.

The reported ExpressTURN connection failed repeatedly near five minutes with
the former 300-second threshold. A 240-second trial on the usual separate client
machine remained connected for approximately 35 minutes 36 seconds, with
successful refreshes and no recorded connection failure. This validates the
chosen interval for that setup without establishing whether the original fault
was in the provider, client permission handling or surrounding network.

The temporary timing environment variable, feature forwarding, renewal trace
collector and investigation fixtures have been removed. Existing isolated TURN
transport and routing regression checks remain in place.
