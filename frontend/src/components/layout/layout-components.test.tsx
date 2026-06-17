import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { HairlineList, HairlineRow } from './hairline-list';
import { LevelsGrid, LevelsGridCell } from './levels-grid';
import { TriPanel, TriPanelCell } from './tri-panel';
import { WindowControl } from './window-control';
import { ViewSwitch } from './view-switch';
import { ModalSheet } from './modal-sheet';

describe('Layout Components Unit Tests', () => {
  // Test 1: HairlineList renders N children as cells
  it('renders N children as cells in HairlineList', () => {
    render(
      <HairlineList>
        <HairlineRow leftContent="row-item-a" rightContent="control-a" />
        <HairlineRow leftContent="row-item-b" rightContent="control-b" />
        <HairlineRow leftContent="row-item-c" rightContent="control-c" />
      </HairlineList>
    );

    expect(screen.getByText('row-item-a')).toBeInTheDocument();
    expect(screen.getByText('row-item-b')).toBeInTheDocument();
    expect(screen.getByText('row-item-c')).toBeInTheDocument();
    expect(screen.getByText('control-a')).toBeInTheDocument();
    expect(screen.getByText('control-b')).toBeInTheDocument();
    expect(screen.getByText('control-c')).toBeInTheDocument();
  });

  // Test 2: HairlineList conforms to the hairline pattern (container bg-line gap-px, cells bg-raise, no borders)
  it('verifies HairlineList conforms to the hairline pattern', () => {
    const { container } = render(
      <HairlineList>
        <HairlineRow leftContent="row-item-a" />
      </HairlineList>
    );

    const listElement = container.firstChild as HTMLElement;
    expect(listElement).toHaveClass('bg-line');
    expect(listElement).toHaveClass('gap-px');
    expect(listElement).toHaveClass('border');

    const cellElement = listElement.querySelector('.bg-raise') as HTMLElement;
    expect(cellElement).toBeInTheDocument();
    // Cells should not have border-line/border-line-2/border classes of their own
    expect(cellElement).not.toHaveClass('border');
    expect(cellElement).not.toHaveClass('border-line');
    expect(cellElement).not.toHaveClass('border-line-2');
  });

  // Test 3: LevelsGrid conforms to the hairline pattern (container bg-line gap-px, cells bg-raise, no borders)
  it('verifies LevelsGrid conforms to the hairline pattern', () => {
    const { container } = render(
      <LevelsGrid>
        <LevelsGridCell eyebrow="kicker" value="value" description="desc" />
      </LevelsGrid>
    );

    const gridElement = container.firstChild as HTMLElement;
    expect(gridElement).toHaveClass('bg-line');
    expect(gridElement).toHaveClass('gap-px');
    expect(gridElement).toHaveClass('border');

    const cellElement = gridElement.querySelector('.bg-raise') as HTMLElement;
    expect(cellElement).toBeInTheDocument();
    expect(cellElement).not.toHaveClass('border');
    expect(cellElement).not.toHaveClass('border-line');
    expect(cellElement).not.toHaveClass('border-line-2');
  });

  // Test 4: TriPanel conforms to the hairline pattern (container bg-line gap-px, cells bg-raise, no borders)
  it('verifies TriPanel conforms to the hairline pattern', () => {
    const { container } = render(
      <TriPanel>
        <TriPanelCell header="header" title="title" body="body" />
      </TriPanel>
    );

    const panelElement = container.firstChild as HTMLElement;
    expect(panelElement).toHaveClass('bg-line');
    expect(panelElement).toHaveClass('gap-px');
    expect(panelElement).toHaveClass('border');

    const cellElement = panelElement.querySelector('.bg-raise') as HTMLElement;
    expect(cellElement).toBeInTheDocument();
    expect(cellElement).not.toHaveClass('border');
    expect(cellElement).not.toHaveClass('border-line');
    expect(cellElement).not.toHaveClass('border-line-2');
  });

  // Test 5: LevelsGrid text children have min-w-0 for the reflow law
  it('verifies LevelsGrid text children have min-w-0', () => {
    render(
      <LevelsGrid>
        <LevelsGridCell eyebrow="kicker" value="value" description="desc" />
      </LevelsGrid>
    );

    const kickerEl = screen.getByText('kicker');
    const valueEl = screen.getByText('value');
    const descEl = screen.getByText('desc');

    expect(kickerEl).toHaveClass('min-w-0');
    expect(valueEl).toHaveClass('min-w-0');
    expect(descEl).toHaveClass('min-w-0');
  });

  // Test 6: TriPanel text children have min-w-0 for the reflow law
  it('verifies TriPanel text children have min-w-0', () => {
    render(
      <TriPanel>
        <TriPanelCell header="header" title="title" body="body" />
      </TriPanel>
    );

    const headerEl = screen.getByText('header');
    const titleEl = screen.getByText('title');
    const bodyEl = screen.getByText('body');

    expect(headerEl).toHaveClass('min-w-0');
    expect(titleEl).toHaveClass('min-w-0');
    expect(bodyEl).toHaveClass('min-w-0');
  });

  // Test 7: WindowControl click changes value and active gets amber classes
  it('handles click changes value and active gets amber classes in WindowControl', async () => {
    const onChange = vi.fn();
    render(<WindowControl value="24h" onChange={onChange} />);

    const activeBtn = screen.getByRole('button', { name: '24h' });
    const inactiveBtn = screen.getByRole('button', { name: 'Live' });

    // Active gets amber classes
    expect(activeBtn).toHaveClass('bg-amber');
    expect(activeBtn).toHaveClass('text-amber-ink');
    expect(activeBtn).toHaveClass('font-semibold');

    // Inactive does not have amber classes
    expect(inactiveBtn).not.toHaveClass('bg-amber');
    expect(inactiveBtn).not.toHaveClass('text-amber-ink');

    // Clicking inactive triggers onChange
    await userEvent.click(inactiveBtn);
    expect(onChange).toHaveBeenCalledWith('Live');
  });

  // Test 8: WindowControl keyboard focus and activation
  it('handles keyboard focus and activation in WindowControl', async () => {
    const onChange = vi.fn();
    render(<WindowControl value="24h" onChange={onChange} />);

    const inactiveBtn = screen.getByRole('button', { name: '1h' });

    // Focus element
    inactiveBtn.focus();
    expect(inactiveBtn).toHaveFocus();

    // Trigger via keyboard Enter
    await userEvent.keyboard('{Enter}');
    expect(onChange).toHaveBeenCalledWith('1h');
  });

  // Test 9: ViewSwitch active segment has stroke/currentColor + text-amber and NO amber fill, and bg-raise-2 active
  it('verifies active segment styling in ViewSwitch', async () => {
    const onChange = vi.fn();
    render(<ViewSwitch value="pipeline" onChange={onChange} />);

    const pipeBtn = screen.getByRole('button', { name: 'Pipeline view' });
    const boardBtn = screen.getByRole('button', { name: 'Board view' });

    expect(pipeBtn).toHaveClass('text-amber');
    expect(pipeBtn).toHaveClass('bg-raise-2');
    expect(pipeBtn).not.toHaveClass('bg-amber'); // no amber fill!

    // Verify SVG has stroke="currentColor" and fill="none"
    const svg = pipeBtn.querySelector('svg');
    expect(svg?.getAttribute('stroke')).toBe('currentColor');
    expect(svg?.getAttribute('fill')).toBe('none');

    // Click board view triggers onChange
    await userEvent.click(boardBtn);
    expect(onChange).toHaveBeenCalledWith('board');
  });

  // Test 10: ModalSheet renders zones with no dialog role of its own
  it('verifies ModalSheet renders zones with no dialog role of its own', () => {
    render(
      <ModalSheet
        title="example-heading"
        meta="example-meta"
        closeButtonSlot={<button>close</button>}
        actionLeftSlot={<span>left</span>}
        actionButtonsSlot={<button>confirm</button>}
      >
        <div>body-content</div>
      </ModalSheet>
    );

    // Header, meta, close, body, and actions are rendered
    expect(screen.getByText('example-heading')).toBeInTheDocument();
    expect(screen.getByText('example-meta')).toBeInTheDocument();
    expect(screen.getByText('close')).toBeInTheDocument();
    expect(screen.getByText('body-content')).toBeInTheDocument();
    expect(screen.getByText('left')).toBeInTheDocument();
    expect(screen.getByText('confirm')).toBeInTheDocument();

    // Verify no role="dialog" is present
    const dialog = screen.queryByRole('dialog');
    expect(dialog).toBeNull();
  });
});
