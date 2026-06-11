Updated the guide and committed it:

```text
5fbea8e Document edge storage and move workflow
```

The step-by-step guide is here:

[docs/edge-nats-prometheus.md](/home/user/avena-rs/docs/edge-nats-prometheus.md)

Useful sections now in that file:

```text
Storage Check
Step 0 through Step 11
Normal Bring-Online Command Set
Normal Turn-Offline Command Set
After Moving The Box
Troubleshooting By Symptom
```

Once OATS access for `nats1`/`nats2` is fixed, do this:

```bash
curl -fsS http://127.0.0.1:8222/leafz | jq '{leafnodes, leafs}'
nats --server nats://nats1.oats:4222 --creds rust-ljm/apt.creds rtt
```

If `leafnodes` is still `0` after a minute:

```bash
sudo systemctl restart nats-leaf
curl -fsS http://127.0.0.1:8222/leafz | jq '{leafnodes, leafs}'
```

Then start collection:

```bash
cd /home/user/avena-rs/rust-ljm
./archiverctl.sh restart
./streamerctl.sh restart
```

To stop before putting it in the car:

```bash
cd /home/user/avena-rs/rust-ljm
./streamerctl.sh stop
sleep 5
./archiverctl.sh stop
sudo systemctl stop alloy nats-exporter nats-leaf
sudo systemctl poweroff
```

To restart at the new location:

```bash
cd /home/user/avena-rs
sudo systemctl start nats-leaf nats-exporter alloy

cd /home/user/avena-rs/rust-ljm
./archiverctl.sh start
./streamerctl.sh start
```

Storage docs now include the 1 TB SSD/LVM expansion check too. `TODO.md` is still your only uncommitted local change; I left it alone.