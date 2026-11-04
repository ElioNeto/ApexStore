import { TestBed } from '@angular/core/testing';
import { AppComponent } from './app.component';
import { appConfig } from './app.config';

describe('Application shell', () => {
  beforeEach(async () => {
    localStorage.setItem('apex_theme', 'dark');
    await TestBed.configureTestingModule({
      imports: [AppComponent],
      providers: appConfig.providers,
    }).compileComponents();
  });

  afterEach(() => {
    localStorage.removeItem('apex_theme');
    document.documentElement.classList.remove('light-mode');
  });

  it('renders navigation and icons with the production providers', async () => {
    const fixture = TestBed.createComponent(AppComponent);
    fixture.autoDetectChanges();
    await fixture.whenStable();
    const element: HTMLElement = fixture.nativeElement;
    expect(element.querySelector('.logo-text')?.textContent).toContain('ApexStore');
    expect(element.querySelector('a[href="/keys"]')?.textContent).toContain('Key Explorer');
    expect(element.querySelector('a[href="/admin"]')?.textContent).toContain('Tokens');
    expect(element.querySelector('.sidebar-logo svg')).not.toBeNull();
  });

  it('updates the visible theme after a browser click', async () => {
    const fixture = TestBed.createComponent(AppComponent);
    fixture.autoDetectChanges();
    await fixture.whenStable();
    const button: HTMLButtonElement = fixture.nativeElement.querySelector('.theme-toggle');
    expect(button.textContent).toContain('Light Mode');
    button.click();
    await fixture.whenStable();
    expect(button.textContent).toContain('Dark Mode');
    expect(document.documentElement.classList.contains('light-mode')).toBeTrue();
    expect(localStorage.getItem('apex_theme')).toBe('light');
    button.click();
    await fixture.whenStable();
    expect(button.textContent).toContain('Light Mode');
    expect(document.documentElement.classList.contains('light-mode')).toBeFalse();
    expect(localStorage.getItem('apex_theme')).toBe('dark');
  });
});
