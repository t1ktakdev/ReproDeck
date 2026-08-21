import { useI18n } from "../i18n";
import type { EnvironmentSnapshot } from "../types";

export function EnvironmentView({ environment }: { environment: EnvironmentSnapshot | null }) {
  const { t } = useI18n();
  return <section className="view-page"><header className="view-heading"><div><h2>{t("environment.title")}</h2><p>{t("environment.description")}</p></div></header>
    {!environment ? <div className="quiet-empty"><h3>{t("environment.empty")}</h3><p>{t("environment.emptyHelp")}</p></div> : <div className="environment-sheet">
      <section><h3>{t("environment.system")}</h3><dl><div><dt>{t("environment.os")}</dt><dd>{environment.os}</dd></div><div><dt>{t("environment.arch")}</dt><dd>{environment.arch}</dd></div><div><dt>Git</dt><dd>{environment.git_version || t("common.none")}</dd></div></dl></section>
      <section><h3>{t("environment.runtimes")}</h3>{Object.keys(environment.runtimes).length === 0 ? <p className="muted-copy">{t("environment.noRuntimes")}</p> : <dl>{Object.entries(environment.runtimes).map(([name, version]) => <div key={name}><dt>{name}</dt><dd>{version}</dd></div>)}</dl>}</section>
    </div>}
  </section>;
}
