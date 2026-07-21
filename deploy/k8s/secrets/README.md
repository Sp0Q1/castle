# Secrets management (Sealed Secrets)

In production the tenant chart creates **no** Secret from Helm values
(`secrets.external: true`). Instead, each tenant's `<release>-app` (and, for an
internal DB, `<release>-db`) Secret is provided out-of-band as a **SealedSecret**:
encrypted to the cluster's controller key, safe to commit to git, decrypted in
cluster by the controller. No plaintext secret ever lives in values, `--set`, or
shell history.

## One-time: install the controller

```bash
kubectl apply -f https://github.com/bitnami-labs/sealed-secrets/releases/download/v0.27.1/controller.yaml
# and install the kubeseal CLI from the same release
```

Back up the controller's private key (its sealing key) — without it, restoring
into a fresh cluster can't decrypt existing SealedSecrets:
```bash
kubectl -n kube-system get secret -l sealedsecrets.bitnami.com/sealed-secrets-key -o yaml > sealed-secrets-key.backup.yaml  # store securely, offline
```

## Per tenant: seal, commit, deploy

```bash
# 1) generate the SealedSecret bundle (values from env; generated if unset)
export OAUTH2_CLIENT_SECRET=... DB_PASSWORD=...            # from your secret source
./seal-tenant.sh castle-ironclad tenant-ironclad --proxy --internal-db > sealed-ironclad.yaml

# 2) commit sealed-ironclad.yaml (it's ciphertext) and apply it
kubectl apply -f sealed-ironclad.yaml                      # controller creates the real Secret(s)

# 3) deploy the tenant referencing them — no plaintext passed
helm upgrade --install castle-ironclad ../tenant -n tenant-ironclad \
  --set secrets.external=true --set codename=ironclad ...  # (+ image/keycloak/etc.)
```

The chart's Deployment/StatefulSet reference `<release>-app` / `<release>-db` by
name in both modes, so switching `secrets.external` on/off changes only *where
the Secret comes from*, not the workload.

## Notes
- A SealedSecret only decrypts on the cluster whose controller sealed it (scoped
  to namespace+name by default), so the committed file is useless if leaked.
- Rotating a secret = re-seal + re-apply + `kubectl rollout restart` the tenant.
- Alternative: **External Secrets Operator** if you already run a cloud secret
  manager / Vault as the source of truth — same `secrets.external: true` contract
  on the chart side, just a different provider creating the Secret.
