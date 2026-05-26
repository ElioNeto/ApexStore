import { TestBed } from '@angular/core/testing';
import { ToastService } from './toast.service';

describe('ToastService', () => {
  let service: ToastService;

  beforeEach(() => {
    TestBed.configureTestingModule({
      providers: [ToastService],
    });
    service = TestBed.inject(ToastService);
  });

  it('should be created', () => {
    expect(service).toBeTruthy();
  });

  it('should start with an empty toast list', () => {
    expect(service.toasts().length).toBe(0);
  });

  describe('show()', () => {
    it('should add a toast with the given message and type', () => {
      service.show('Hello world', 'success');
      const toasts = service.toasts();
      expect(toasts.length).toBe(1);
      expect(toasts[0].message).toBe('Hello world');
      expect(toasts[0].type).toBe('success');
    });

    it('should assign incrementing IDs', () => {
      service.show('First');
      service.show('Second');
      const toasts = service.toasts();
      expect(toasts[0].id).toBe(0);
      expect(toasts[1].id).toBe(1);
    });

    it('should default type to info', () => {
      service.show('Info toast');
      expect(service.toasts()[0].type).toBe('info');
    });
  });

  describe('dismiss()', () => {
    it('should remove the toast with the given ID', () => {
      service.show('Toast A');
      service.show('Toast B');
      expect(service.toasts().length).toBe(2);

      service.dismiss(0);
      expect(service.toasts().length).toBe(1);
      expect(service.toasts()[0].message).toBe('Toast B');
    });

    it('should do nothing if the ID does not exist', () => {
      service.show('Only toast');
      service.dismiss(99);
      expect(service.toasts().length).toBe(1);
    });
  });

  describe('helper methods', () => {
    it('success() should add a success toast', () => {
      service.success('OK!');
      expect(service.toasts()[0].type).toBe('success');
    });

    it('error() should add an error toast', () => {
      service.error('Fail!');
      expect(service.toasts()[0].type).toBe('error');
    });

    it('info() should add an info toast', () => {
      service.info('Note');
      expect(service.toasts()[0].type).toBe('info');
    });
  });

  describe('auto-dismiss', () => {
    beforeEach(() => {
      jasmine.clock().install();
    });

    afterEach(() => {
      jasmine.clock().uninstall();
    });

    it('should remove toast after the default duration (3500ms)', () => {
      service.show('Auto dismiss');
      expect(service.toasts().length).toBe(1);

      jasmine.clock().tick(3500);
      expect(service.toasts().length).toBe(0);
    });

    it('should remove toast after a custom duration', () => {
      service.show('Custom duration', 'info', 1000);
      expect(service.toasts().length).toBe(1);

      jasmine.clock().tick(999);
      expect(service.toasts().length).toBe(1);

      jasmine.clock().tick(1);
      expect(service.toasts().length).toBe(0);
    });
  });
});
