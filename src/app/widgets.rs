//! Widgets desenhados à mão com o painter: barra de título, caixa de
//! concluir, lixeira, alça de arrastar, engrenagem, olho do filtro e alças
//! de redimensionar (a janela não tem borda nativa, e os glifos 🗑/⚙/⠿ não
//! existem nas fontes padrão do egui).

use eframe::egui::{
    self, Align2, CursorIcon, FontId, Id, PointerButton, Pos2, Rect, ResizeDirection, Sense,
    Stroke, Vec2, ViewportCommand, pos2, vec2,
};

use crate::tema::cor;

const ALTURA_BARRA: f32 = 24.0;
/// Espessura das alças invisíveis de redimensionamento nas bordas.
const BORDA_RESIZE: f32 = 6.0;
/// Largura reservada para o botão de apagar (sempre reservada, para o texto
/// não "pular" quando o botão aparece no hover).
pub(super) const LARGURA_APAGAR: f32 = 20.0;

/// Largura reservada para a alça de arrastar (sempre, como a da lixeira).
pub(super) const LARGURA_ALCA: f32 = 12.0;

/// Alça ⠿ para reordenar: reserva o espaço sempre e só desenha os pontos
/// quando `visivel`. Id explícito pela tarefa: o egui acompanha o arrasto
/// pelo id, e o automático mudaria se a lista andasse durante o arrasto.
pub(super) fn alca_arrastar(ui: &mut egui::Ui, id: Id, visivel: bool) -> egui::Response {
    let tamanho = vec2(LARGURA_ALCA, ui.spacing().interact_size.y);
    let (r, _) = ui.allocate_exact_size(tamanho, Sense::hover());
    let resp = ui.interact(r, id, Sense::drag());
    let ativa = resp.dragged() || resp.hovered();
    if !visivel && !ativa {
        return resp;
    }
    let resp = resp.on_hover_cursor(CursorIcon::Grab);
    let cor = if ativa {
        cor::AZUL
    } else {
        ui.visuals().weak_text_color()
    };
    let c = r.center();
    for dx in [-2.0, 2.0] {
        for dy in [-4.0, 0.0, 4.0] {
            ui.painter()
                .circle_filled(pos2(c.x + dx, c.y + dy), 1.1, cor);
        }
    }
    resp
}

/// Caixa de concluir no estilo do tema: contorno discreto, ✓ verde quando
/// feita. Focável pelo Tab; Espaço/Enter contam como clique.
pub(super) fn caixa(ui: &mut egui::Ui, feita: bool) -> egui::Response {
    let tamanho = Vec2::splat(ui.spacing().interact_size.y);
    let (r, resp) = ui.allocate_exact_size(tamanho, Sense::CLICK | Sense::FOCUSABLE);
    let quadro = Rect::from_center_size(r.center(), Vec2::splat(12.0));
    let borda = if resp.has_focus() || resp.hovered() {
        cor::AZUL
    } else if feita {
        cor::CINZA
    } else {
        cor::TEXTO_FRACO
    };
    let p = ui.painter();
    p.rect_stroke(quadro, 2, Stroke::new(1.0, borda), egui::StrokeKind::Inside);
    if feita {
        let c = quadro.center();
        p.line(
            vec![
                pos2(c.x - 3.5, c.y),
                pos2(c.x - 1.0, c.y + 2.5),
                pos2(c.x + 3.5, c.y - 2.5),
            ],
            Stroke::new(1.5, cor::VERDE),
        );
    }
    resp
}

/// Lixeira desenhada com o painter (o glifo 🗑 não existe nas fontes padrão
/// do egui). Invisível, só reserva o espaço (sem desenho nem dica). Devolve `true`
/// no clique quando visível.
pub(super) fn botao_lixeira(ui: &mut egui::Ui, visivel: bool) -> bool {
    // Sempre a mesma alocação (visível ou não), para os ids automáticos dos
    // widgets seguintes não mudarem com o hover e tirarem o foco de uma edição.
    let tamanho = vec2(LARGURA_APAGAR, ui.spacing().interact_size.y);
    // Sem FOCUSABLE: uma lixeira (mesmo invisível) não pode ser parada do Tab.
    let (r, resp) = ui.allocate_exact_size(tamanho, Sense::CLICK);
    if !visivel {
        return false;
    }
    let resp = resp.on_hover_text("Apagar");
    let cor = if resp.hovered() {
        cor::VERMELHO
    } else {
        ui.visuals().weak_text_color()
    };
    let traco = Stroke::new(1.5, cor);
    let p = ui.painter();
    let c = r.center();
    // Ícone de ~12 px: tampa, alça, corpo afunilado e dois frisos.
    let tampa = c.y - 4.0;
    p.line_segment([pos2(c.x - 6.0, tampa), pos2(c.x + 6.0, tampa)], traco);
    p.line_segment(
        [pos2(c.x - 2.0, tampa - 2.0), pos2(c.x + 2.0, tampa - 2.0)],
        traco,
    );
    let (topo, base) = (tampa + 1.5, c.y + 6.0);
    p.add(egui::Shape::closed_line(
        vec![
            pos2(c.x - 4.5, topo),
            pos2(c.x + 4.5, topo),
            pos2(c.x + 3.5, base),
            pos2(c.x - 3.5, base),
        ],
        traco,
    ));
    for dx in [-1.5, 1.5] {
        p.line_segment(
            [pos2(c.x + dx, topo + 2.0), pos2(c.x + dx, base - 2.0)],
            Stroke::new(1.0, cor),
        );
    }
    resp.clicked()
}

pub(super) enum Botao {
    Fechar,
    Config,
    Filtro,
}

/// Barra própria: arrastar move a janela. Devolve o botão clicado.
/// `esconder_concluidas` só desenha o estado do olho do filtro (que não
/// aparece com as configurações abertas).
pub(super) fn barra_titulo(
    ui: &mut egui::Ui,
    pendentes: usize,
    config_aberta: bool,
    esconder_concluidas: bool,
) -> Option<Botao> {
    let (barra, arrasto) =
        ui.allocate_exact_size(vec2(ui.available_width(), ALTURA_BARRA), Sense::DRAG);
    if arrasto.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }

    let fonte = FontId::proportional(14.0);
    let titulo = if config_aberta {
        "configurações"
    } else {
        "tarefas"
    };
    let escrito = ui.painter().text(
        barra.left_center(),
        Align2::LEFT_CENTER,
        titulo,
        fonte.clone(),
        cor::TEXTO,
    );
    if !config_aberta && pendentes > 0 {
        ui.painter().text(
            escrito.right_center() + vec2(8.0, 0.0),
            Align2::LEFT_CENTER,
            pendentes.to_string(),
            fonte,
            cor::AZUL,
        );
    }

    let botao = Rect::from_min_size(
        barra.right_top() - vec2(ALTURA_BARRA, 0.0),
        Vec2::splat(ALTURA_BARRA),
    );
    let fechar = ui
        .interact(botao, Id::unique("fechar"), Sense::CLICK)
        .on_hover_text("Esconder");
    let cor_x = if fechar.hovered() {
        cor::VERMELHO
    } else {
        ui.visuals().weak_text_color()
    };
    let x = botao.shrink(8.0);
    let traco = Stroke::new(1.5, cor_x);
    ui.painter()
        .line_segment([x.left_top(), x.right_bottom()], traco);
    ui.painter()
        .line_segment([x.right_top(), x.left_bottom()], traco);

    let engrenagem = botao.translate(vec2(-ALTURA_BARRA, 0.0));
    let dica = if config_aberta {
        "Voltar à lista"
    } else {
        "Configurações"
    };
    // Sense::CLICK (sem FOCUSABLE): não vira parada do Tab.
    let config = ui
        .interact(engrenagem, Id::unique("config"), Sense::CLICK)
        .on_hover_text(dica);
    desenhar_engrenagem(ui, engrenagem.center(), config.hovered() || config_aberta);

    let filtro = (!config_aberta).then(|| {
        let olho = engrenagem.translate(vec2(-ALTURA_BARRA, 0.0));
        let dica = if esconder_concluidas {
            "Mostrar concluídas"
        } else {
            "Esconder concluídas"
        };
        let resp = ui
            .interact(olho, Id::unique("filtro"), Sense::CLICK)
            .on_hover_text(dica);
        desenhar_olho(ui, olho.center(), esconder_concluidas, resp.hovered());
        resp
    });

    if fechar.clicked() {
        Some(Botao::Fechar)
    } else if config.clicked() {
        Some(Botao::Config)
    } else if filtro.is_some_and(|r| r.clicked()) {
        Some(Botao::Filtro)
    } else {
        None
    }
}

/// Olho de ~14 px; riscado (e em destaque) quando as concluídas estão escondidas.
fn desenhar_olho(ui: &egui::Ui, c: Pos2, riscado: bool, hover: bool) {
    let cor = if riscado || hover {
        cor::AZUL
    } else {
        ui.visuals().weak_text_color()
    };
    let traco = Stroke::new(1.5, cor);
    let p = ui.painter();
    let contorno = (0..16)
        .map(|i| {
            let ang = i as f32 * std::f32::consts::TAU / 16.0;
            pos2(c.x + 7.0 * ang.cos(), c.y + 4.0 * ang.sin())
        })
        .collect();
    p.add(egui::Shape::closed_line(contorno, traco));
    p.circle_filled(c, 2.0, cor);
    if riscado {
        p.line_segment([c + vec2(-6.0, 6.0), c + vec2(6.0, -6.0)], traco);
    }
}

/// Engrenagem de ~14 px desenhada com o painter (sem depender de glifo).
fn desenhar_engrenagem(ui: &egui::Ui, c: Pos2, destaque: bool) {
    let cor = if destaque {
        cor::AZUL
    } else {
        ui.visuals().weak_text_color()
    };
    let p = ui.painter();
    p.circle_stroke(c, 3.5, Stroke::new(1.5, cor));
    for i in 0..8 {
        let ang = i as f32 * std::f32::consts::FRAC_PI_4;
        let dir = vec2(ang.cos(), ang.sin());
        p.line_segment([c + dir * 4.5, c + dir * 7.0], Stroke::new(2.0, cor));
    }
}

/// Alças invisíveis nas 4 bordas e 4 cantos (a janela sem borda não tem resize nativo).
pub(super) fn alcas_redimensionar(ui: &egui::Ui, r: Rect) {
    let b = BORDA_RESIZE;
    let (l, t, rt, bt) = (r.left(), r.top(), r.right(), r.bottom());
    let alcas = [
        (
            ResizeDirection::North,
            Rect::from_min_max(pos2(l + b, t), pos2(rt - b, t + b)),
            CursorIcon::ResizeNorth,
        ),
        (
            ResizeDirection::South,
            Rect::from_min_max(pos2(l + b, bt - b), pos2(rt - b, bt)),
            CursorIcon::ResizeSouth,
        ),
        (
            ResizeDirection::West,
            Rect::from_min_max(pos2(l, t + b), pos2(l + b, bt - b)),
            CursorIcon::ResizeWest,
        ),
        (
            ResizeDirection::East,
            Rect::from_min_max(pos2(rt - b, t + b), pos2(rt, bt - b)),
            CursorIcon::ResizeEast,
        ),
        (
            ResizeDirection::NorthWest,
            Rect::from_min_max(pos2(l, t), pos2(l + b, t + b)),
            CursorIcon::ResizeNorthWest,
        ),
        (
            ResizeDirection::NorthEast,
            Rect::from_min_max(pos2(rt - b, t), pos2(rt, t + b)),
            CursorIcon::ResizeNorthEast,
        ),
        (
            ResizeDirection::SouthWest,
            Rect::from_min_max(pos2(l, bt - b), pos2(l + b, bt)),
            CursorIcon::ResizeSouthWest,
        ),
        (
            ResizeDirection::SouthEast,
            Rect::from_min_max(pos2(rt - b, bt - b), pos2(rt, bt)),
            CursorIcon::ResizeSouthEast,
        ),
    ];
    for (i, (direcao, rect, cursor)) in alcas.into_iter().enumerate() {
        let resp = ui
            .interact(rect, Id::unique(("alca_resize", i)), Sense::drag())
            .on_hover_cursor(cursor);
        if resp.drag_started_by(PointerButton::Primary) {
            ui.ctx()
                .send_viewport_cmd(ViewportCommand::BeginResize(direcao));
        }
    }
}
