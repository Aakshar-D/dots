import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { toast } from "sonner";

import { commands } from "@/api/commands";
import type { PairingView, PhoneDevice } from "@/api/types";
import { ConfirmButton } from "@/components/ConfirmButton";
import { ErrorNote } from "@/components/ErrorNote";
import { Button } from "@/components/ui/button";
import { localDateTime, relativeTime } from "@/lib/format";
import { notifyError } from "@/lib/notify";
import { invalidate, keys } from "@/lib/query";

function Section(props: { title: string; children: ReactNode }) {
  return (
    <fieldset className="flex max-w-3xl flex-col gap-3 rounded-lg border p-4">
      <legend className="px-1 text-sm font-semibold">{props.title}</legend>
      {props.children}
    </fieldset>
  );
}

/// The QR code of a pairing link, drawn from the module path the core made: no image is
/// fetched and no markup is injected.
function QrCode({ pairing }: { pairing: PairingView }) {
  const size = pairing.qr_size;
  return (
    <svg
      role="img"
      aria-label="QR code to pair a phone"
      viewBox={`0 0 ${size} ${size}`}
      className="size-56 rounded-md bg-white"
      shapeRendering="crispEdges"
    >
      <rect width={size} height={size} fill="#fff" />
      <path d={pairing.qr_path} fill="#000" />
    </svg>
  );
}

/// A pairing in progress: the QR code, the code to type, the address and the certificate's
/// fingerprint to compare on the phone.
function Pairing({ pairing, onCancel }: { pairing: PairingView; onCancel: () => void }) {
  return (
    <div className="flex flex-col gap-3 rounded-md border p-3" aria-label="Pairing">
      <div className="flex flex-wrap items-start gap-4">
        <QrCode pairing={pairing} />
        <ol className="flex max-w-sm list-decimal flex-col gap-2 pl-5 text-sm">
          <li>Scan the code with your phone's camera, on the same Wi-Fi as this PC.</li>
          <li>
            Your phone's browser warns that the connection is not private, because Sage made its
            own certificate. Check that the certificate's fingerprint is the one below, then
            continue to the page.
          </li>
          <li>Tap Pair. This PC then asks whether the phone may pair; check the number.</li>
        </ol>
      </div>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1 text-xs">
        <dt className="text-muted-foreground">Or open</dt>
        <dd className="font-mono break-all">{pairing.address}</dd>
        <dt className="text-muted-foreground">and type</dt>
        <dd className="font-mono text-base font-semibold tracking-widest" aria-label="Pairing code">
          {pairing.code}
        </dd>
        <dt className="text-muted-foreground">Fingerprint</dt>
        <dd className="font-mono break-all" aria-label="Certificate fingerprint">
          {pairing.fingerprint}
        </dd>
        <dt className="text-muted-foreground">Good until</dt>
        <dd>{localDateTime(pairing.expires_at)}, once</dd>
      </dl>
      <div>
        <Button type="button" size="sm" variant="outline" onClick={onCancel}>
          Cancel pairing
        </Button>
      </div>
    </div>
  );
}

function Device({ device }: { device: PhoneDevice }) {
  const qc = useQueryClient();
  const remove = useMutation({
    mutationFn: () => commands.phoneDeviceRemove(device.id),
    onSuccess: () => {
      invalidate(qc, keys.phone);
      toast.success(`${device.name} removed`);
    },
    onError: notifyError,
  });
  return (
    <li aria-label={device.name} className="flex items-center gap-3 rounded-md border p-2">
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{device.name}</p>
        <p className="text-xs text-muted-foreground">
          Paired {relativeTime(device.created_at)}
          {device.last_seen_at && ` · last seen ${relativeTime(device.last_seen_at)}`}
          {device.last_ip && ` from ${device.last_ip}`}
        </p>
      </div>
      <ConfirmButton
        title={`Remove ${device.name}?`}
        description="It can no longer see or answer your Inbox, from its next request on. You can pair it again."
        confirmLabel="Remove"
        ariaLabel={`Remove ${device.name}`}
        disabled={remove.isPending}
        onConfirm={() => remove.mutate()}
      >
        Remove
      </ConfirmButton>
    </li>
  );
}

/// Settings > Phone, below the switch and port: where the listener stands, pairing, the paired
/// phones and what they did. `stored` is the saved switch: the listener follows it, not the
/// unsaved draft.
export function PhoneSection({ stored }: { stored: boolean }) {
  const qc = useQueryClient();
  const [pairing, setPairing] = useState<PairingView | null>(null);
  const [address, setAddress] = useState<string | null>(null);
  const status = useQuery({
    queryKey: keys.phoneStatus,
    queryFn: commands.phoneStatus,
    // While a pairing is shown, a phone may pair any moment.
    refetchInterval: pairing ? 2000 : false,
  });
  const devices = useQuery({
    queryKey: keys.phoneDevices,
    queryFn: commands.phoneDevices,
    refetchInterval: pairing ? 2000 : false,
  });
  const actions = useQuery({
    queryKey: keys.phoneActions,
    queryFn: commands.phoneActions,
    refetchInterval: pairing ? 2000 : false,
  });
  // The switch was saved: the listener started or stopped.
  useEffect(() => {
    invalidate(qc, keys.phone);
    if (!stored) setPairing(null);
  }, [qc, stored]);
  // A phone that pairs while the code shows ends the pairing view.
  const seen = useRef<number | null>(null);
  const count = devices.data?.length;
  useEffect(() => {
    if (count === undefined) return;
    if (pairing && seen.current !== null && count > seen.current) {
      setPairing(null);
      toast.success("Phone paired");
    }
    seen.current = count;
  }, [count, pairing]);
  // A pairing that ran out, or that the core forgot (turned off and on), is not shown.
  const pairingLive = status.data?.pairing;
  useEffect(() => {
    if (pairing && pairingLive === false) setPairing(null);
  }, [pairing, pairingLive]);

  const start = useMutation({
    mutationFn: () => commands.phonePairStart(address),
    onSuccess: (view) => {
      seen.current = devices.data?.length ?? 0;
      // The status read before the pairing says none waits: it must not end this one.
      qc.setQueryData(keys.phoneStatus, (old: typeof status.data) =>
        old ? { ...old, pairing: true } : old,
      );
      setPairing(view);
    },
    onError: notifyError,
  });
  const cancel = useMutation({
    mutationFn: commands.phonePairCancel,
    onSettled: () => {
      setPairing(null);
      invalidate(qc, keys.phoneStatus);
    },
  });

  const s = status.data;
  return (
    <>
      {stored && (
        <Section title="Pair a phone">
          {status.isError && <ErrorNote error={status.error} />}
          {s && !s.running && (
            <p role="alert" className="text-sm text-destructive">
              Not listening{s.error ? `: ${s.error}` : "."}
            </p>
          )}
          {s?.running && (
            <>
              <p className="text-xs text-muted-foreground">
                Listening on {s.addresses.join(", ")}.
                {s.error && ` ${s.error}`}
              </p>
              {s.fingerprint && (
                <p className="text-xs">
                  <span className="text-muted-foreground">Certificate fingerprint (SHA-256): </span>
                  <span className="font-mono break-all">{s.fingerprint}</span>
                </p>
              )}
              {pairing ? (
                <Pairing pairing={pairing} onCancel={() => cancel.mutate()} />
              ) : (
                <div className="flex flex-wrap items-center gap-2">
                  {s.addresses.length > 1 && (
                    <select
                      aria-label="Address for the QR code"
                      className="h-8 rounded-md border bg-background px-2 text-sm"
                      value={address ?? s.addresses[0]}
                      onChange={(e) => setAddress(e.target.value)}
                    >
                      {s.addresses.map((a) => (
                        <option key={a} value={a}>
                          {a}
                        </option>
                      ))}
                    </select>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    disabled={start.isPending}
                    onClick={() => start.mutate()}
                  >
                    Pair a phone
                  </Button>
                </div>
              )}
            </>
          )}
        </Section>
      )}

      <Section title="Paired phones">
        {devices.isError && <ErrorNote error={devices.error} />}
        {devices.data?.length === 0 && (
          <p className="text-xs text-muted-foreground">No phones are paired.</p>
        )}
        {devices.data && devices.data.length > 0 && (
          <ul className="flex flex-col gap-2">
            {devices.data.map((d) => (
              <Device key={d.id} device={d} />
            ))}
          </ul>
        )}
      </Section>

      {actions.data && actions.data.length > 0 && (
        <Section title="Recent actions from phones">
          <ul aria-label="Recent actions from phones" className="flex flex-col gap-1 text-xs">
            {actions.data.map((a, i) => (
              <li key={`${a.at}-${i}`}>
                <span className="font-medium">{a.device_name}</span>: {a.summary}{" "}
                <span className="text-muted-foreground">· {relativeTime(a.at)}</span>
              </li>
            ))}
          </ul>
        </Section>
      )}
    </>
  );
}
