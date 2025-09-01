# Security Model for NATS Hub + Edge Deployment

This document explains where the system is **secure** and where it’s still **vulnerable** based on my limited knowledge  given the current setup (TLS everywhere, bcrypt passwords, offline CA, non-root services).

---

##  Safe Scenarios

### 1. **Network snooping**

* Hub ↔ Edge ↔ Developer traffic runs over **TLS with mutual authentication** (mTLS).
* Attackers sniffing the network (LAN, Tailscale, Internet) see only encrypted gibberish.

---

### 2. **Man-in-the-Middle (MITM)**

* Both sides prove identity with **CA-signed TLS certificates**.
* A fake hub or edge without a CA-signed cert is immediately rejected.

---

### 3. **Password theft alone**

* Dev passwords are **bcrypt-hashed** in the hub config.
* Even if a password leaks, the attacker also needs the signed cert.
* Password-only = useless.

---

### 4. **Cert theft alone**

* If attacker steals a dev’s or edge’s cert+key but not the password → useless.
* Both factors (cert+password) are required.

---

### 5. **Compromised single developer**

* Only that dev’s cert+account is exposed.
* You can revoke with `remove_dev.sh` without affecting others.

---

### 6. **Compromised single edge**

* Only that edge’s cert+account is exposed.
* Attacker can impersonate that edge only.
* Revoke the edge cert/account, others remain safe.

---

### 7. **Password brute force**

* Bcrypt slows down offline guessing drastically.
* With strong passwords, brute forcing is impractical.

---

### 8. **Rogue clients**

* Hub rejects any connection not signed by the **offline CA**.
* Both TLS cert and password are required.

---

### 9. **Password reuse**

* Even if a password is reused elsewhere and leaks, the attacker still needs the cert.

---

### 10. **Hub or edge service exploit**

* All NATS services run as **non-root** (`nats`, `sampler`, `archiver`).
* Exploits only compromise those limited accounts, not the whole system.

---

### 11. **Hub impersonation**

* Clients verify hub cert against the **offline CA**.
* A fake hub with a self-signed cert is rejected.

---

## Possible Unsafe  Scenarios

### 1. **Root compromise on hub**

* Attacker can:

  * Steal hub’s private key.
  * Read all developer accounts + certs.
  * Access JetStream data.
  
* But: cannot mint new certs (since **CA key is offline**).

---

### 2. **CA key stolen**

* If the offline CA private key (`ca.key`) is leaked:

  * Attacker can mint unlimited fake certs.
  * Entire trust model collapses.
  
* This is the **most critical single secret (but safe with me)**.

---

### 3. **Root compromise on edge**

* Attacker can:

  * Steal edge cert+key.
  * Steal leaf password.
  * Access local JetStream + parquet logs.
* Blast radius limited to that edge until revoked.

---

### 4. **Root compromise on dev laptop**

* Attacker can steal cert+key + password.
* Full impersonation of that dev.
* Must revoke quickly.

---

### 6. **Denial of Service (DoS)**

* TLS handshakes are CPU-heavy.
* Hub can be overwhelmed by floods of connections.
* Fix: firewall rate-limiting or put hub behind reverse proxy/load balancer.

