# Security policy

## Supported versions

Security fixes go into the latest release of Voltrus SCADA Core. Please update
before reporting, and say which version you tested.

## Reporting a vulnerability

Do not open a public issue for a security problem.

* Preferred: use GitHub's private reporting on this repository
  (Security tab, "Report a vulnerability").
* Or email **support@voltrus.id** with the subject `SECURITY`.

Please include the version, how to reproduce, and the impact you see. If the
report involves a live system, do not access data that is not yours.

We aim to acknowledge reports within five business days, keep you updated on
the fix, and credit you in the release notes if you want. We ask for time to
ship a fix before details are made public.

## Scope

In scope: the Core server, the web client, the SDKs and the default
configuration in this repository. The Enterprise add-on is closed source;
report problems in it through the same channels.

Out of scope: findings that need an already compromised host, social
engineering, denial of service by sending very large traffic volumes, and
issues in third-party dependencies that are already tracked upstream (tell us
anyway if we ship a vulnerable version).

## Hardening notes for operators

* Set `auth_secret` to a long random value in the config file.
* Run behind a TLS-terminating reverse proxy and set `cookie_secure` and
  `cors_origin` accordingly.
* Change the generated admin password on first sign-in, and restrict who can
  reach the server port. Industrial networks should not be exposed to the
  internet.
