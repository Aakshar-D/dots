import { useMutation } from "@tanstack/react-query";

import { commands } from "@/api/commands";
import type { Dot, DotSpec } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { sitesOf, withoutSite } from "@/lib/browser";
import { notifyError } from "@/lib/notify";

/// The dot's browser (browser spec section 7): the switch, the sites it may act on without
/// asking, and, for a stored dot, "Open the browser" so the person can log in ahead of time.
/// Removing a site changes the form; Save stores it.
export function BrowserSection(props: {
  spec: DotSpec;
  existing: Dot | null;
  onChange: (spec: DotSpec) => void;
}) {
  const { spec, existing } = props;
  const open = useMutation({ mutationFn: commands.browserOpen, onError: notifyError });
  const sites = sitesOf(spec.policy);
  return (
    <>
      <div className="flex items-center gap-2">
        <Switch
          id="dot-browser"
          checked={spec.browser}
          onCheckedChange={(browser) => props.onChange({ ...spec, browser })}
        />
        <Label htmlFor="dot-browser">Browser</Label>
      </div>
      <p className="-mt-2 text-xs text-muted-foreground">
        Opens a visible Microsoft Edge window with this dot's own profile.
      </p>
      {spec.browser && (
        <>
          <h3 id="dot-browser-sites" className="text-sm font-medium">
            Sites
          </h3>
          {sites.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No site is approved yet: the dot asks before it opens a site or acts on one, and an
              approval can always allow the site.
            </p>
          ) : (
            <ul aria-labelledby="dot-browser-sites" className="flex flex-col gap-1">
              {sites.map(({ site, risky }) => (
                <li key={site} aria-label={site} className="flex items-center gap-2 text-sm">
                  <span className="font-mono">{site}</span>
                  {risky && (
                    <span className="text-xs text-muted-foreground">risky actions allowed</span>
                  )}
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    className="ml-auto"
                    onClick={() =>
                      props.onChange({ ...spec, policy: withoutSite(spec.policy, site) })
                    }
                  >
                    Remove
                  </Button>
                </li>
              ))}
            </ul>
          )}
          {existing ? (
            <div>
              <Button
                type="button"
                size="sm"
                variant="outline"
                disabled={open.isPending}
                onClick={() => open.mutate(existing.id)}
              >
                Open the browser
              </Button>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">Save the dot to open its browser.</p>
          )}
        </>
      )}
    </>
  );
}
