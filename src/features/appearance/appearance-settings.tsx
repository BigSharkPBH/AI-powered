import { Monitor, Moon, Sun } from "lucide-react";
import { setThemePreference, useThemePreference } from "./theme";

const options = [
  { value: "system", label: "跟随系统", description: "与系统外观保持一致", Icon: Monitor },
  { value: "light", label: "浅色", description: "明亮、清晰的工作空间", Icon: Sun },
  { value: "dark", label: "深色", description: "柔和、专注的深色界面", Icon: Moon },
] as const;

export function AppearanceSettings() {
  const preference = useThemePreference();
  return (
    <section className="appearance-settings" aria-labelledby="appearance-heading">
      <div className="section-heading"><h2 id="appearance-heading">外观</h2><p className="muted">选择适合你的工作环境，偏好会保存在本机。</p></div>
      <fieldset className="theme-options">
        <legend className="sr-only">界面主题</legend>
        {options.map(({ value, label, description, Icon }) => (
          <label className="theme-option" data-selected={preference === value} key={value}>
            <input type="radio" name="theme" aria-label={label} value={value} checked={preference === value} onChange={() => setThemePreference(value)} />
            <span className="theme-preview" data-preview={value} aria-hidden="true"><span className="theme-preview-nav"><i /><i /><i /></span><span className="theme-preview-main"><i /><i /><i /><span /></span></span>
            <span className="theme-option-label"><Icon size={16} aria-hidden="true" />{label}</span>
            <span className="theme-option-description">{description}</span>
          </label>
        ))}
      </fieldset>
      <p className="appearance-note muted">外观设置立即生效；选择「跟随系统」时会自动响应系统主题变化。</p>
    </section>
  );
}
