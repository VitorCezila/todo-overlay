"""Gera assets/icon.ico (quadrado arredondado azul com um check escuro).

Sem dependências: escreve PNGs com zlib e empacota num .ico multi-tamanho.
Uso: python assets/gerar_icone.py
"""

import struct
import zlib
from pathlib import Path

TAMANHOS = [16, 20, 24, 32, 40, 48, 64, 256]
AMOSTRAS = 4  # supersampling por eixo, para antialiasing

AZUL = (0x61, 0xAF, 0xEF)  # tema::AZUL (One Half Dark)
FUNDO = (0x28, 0x2C, 0x34)  # tema::FUNDO


def dentro_arredondado(x, y, lado, raio):
    cx = min(max(x, raio), lado - raio)
    cy = min(max(y, raio), lado - raio)
    return (x - cx) ** 2 + (y - cy) ** 2 <= raio * raio


def dist_segmento(p, a, b):
    dx, dy = b[0] - a[0], b[1] - a[1]
    t = ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / (dx * dx + dy * dy)
    t = min(max(t, 0.0), 1.0)
    qx, qy = a[0] + t * dx, a[1] + t * dy
    return ((p[0] - qx) ** 2 + (p[1] - qy) ** 2) ** 0.5


def desenhar(n):
    lado = float(n)
    raio = lado * 0.22
    a, b, c = (lado * 0.26, lado * 0.52), (lado * 0.43, lado * 0.69), (lado * 0.75, lado * 0.33)
    espessura = lado * 0.085
    linhas = []
    for y in range(n):
        linha = bytearray([0])  # filtro PNG "None"
        for x in range(n):
            cobertura = check = 0
            for sy in range(AMOSTRAS):
                for sx in range(AMOSTRAS):
                    p = (x + (sx + 0.5) / AMOSTRAS, y + (sy + 0.5) / AMOSTRAS)
                    if dentro_arredondado(p[0], p[1], lado, raio):
                        cobertura += 1
                        if min(dist_segmento(p, a, b), dist_segmento(p, b, c)) <= espessura:
                            check += 1
            total = AMOSTRAS * AMOSTRAS
            if cobertura == 0:
                linha += bytes(4)
                continue
            f = check / cobertura
            cor = [round(AZUL[i] * (1 - f) + FUNDO[i] * f) for i in range(3)]
            linha += bytes(cor + [round(255 * cobertura / total)])
        linhas.append(bytes(linha))
    return b"".join(linhas)


def png(n):
    def bloco(tipo, dados):
        corpo = tipo + dados
        return struct.pack(">I", len(dados)) + corpo + struct.pack(">I", zlib.crc32(corpo))

    ihdr = struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0)  # 8 bits, RGBA
    return (
        b"\x89PNG\r\n\x1a\n"
        + bloco(b"IHDR", ihdr)
        + bloco(b"IDAT", zlib.compress(desenhar(n), 9))
        + bloco(b"IEND", b"")
    )


def main():
    imagens = [png(n) for n in TAMANHOS]
    cabecalho = struct.pack("<HHH", 0, 1, len(imagens))
    deslocamento = 6 + 16 * len(imagens)
    entradas = b""
    for n, img in zip(TAMANHOS, imagens):
        lado = 0 if n >= 256 else n  # 0 significa 256 no formato .ico
        entradas += struct.pack("<BBBBHHII", lado, lado, 0, 0, 1, 32, len(img), deslocamento)
        deslocamento += len(img)
    destino = Path(__file__).with_name("icon.ico")
    destino.write_bytes(cabecalho + entradas + b"".join(imagens))
    print(f"{destino} ({destino.stat().st_size} bytes)")


if __name__ == "__main__":
    main()
