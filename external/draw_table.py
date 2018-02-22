#!/usr/bin/env python3
"""Utility to draw .png with score table."""
import argparse
import json
from PIL import ImageDraw, Image, ImageFont

class TableDrawer(object):
    FONT_NAME = 'arial'
    TOPIC_WIDTH = 800
    SCORE_WIDTH = 100
    ROW_HEIGHT = 60
    LINE_WIDTH = 3
    CELL_OFFSET = 7
    LINE_COLOR = (100, 80, 20)
    BG_COLOR = (30, 30, 100)
    TEXT_COLOR = (255, 238, 173)

    def __init__(self):
        self._topics = None
        self._scores = None
        self._rows = None
        self._width = None
        self._height = None
        self._font = None
        self._image = None
        self._draw_proxy = None
        self._xs = []
        self._ys = []

    def _parse_data(self, data):
        self._scores = [str(x).strip() for x in data['scores']]
        self._topics = []
        self._rows = []
        for row in data['data']:
            self._topics.append(row['name'])
            self._rows.append([row['name']])

            has_scores = set()
            for score in row['questions']:
                score = str(score).strip()
                if score not in self._scores:
                    raise KeyError(
                        "Question in topic '{}' has score '{}', "
                        "which is not present in scores".format(
                            row['name'],
                            score
                        )
                    )
                has_scores.add(score)

            for score in self._scores:
                if score in has_scores:
                    self._rows[-1].append(score)
                else:
                    self._rows[-1].append('')

    def _define_geometry(self):
        self._width = self.TOPIC_WIDTH + self.SCORE_WIDTH * len(self._scores) + self.LINE_WIDTH
        self._height = self.ROW_HEIGHT * len(self._topics) + self.LINE_WIDTH

    def _define_image(self):
        self._image = Image.new('RGB', (self._width, self._height), self.BG_COLOR)
        self._draw_proxy = ImageDraw.Draw(self._image)

    @staticmethod
    def get_font_size(draw_proxy, font_name, text, width, height):
        """Get maximal font size to fit (width, height) box with given text."""
        size = height
        font = ImageFont.truetype(font_name, size)
        text_size = draw_proxy.textsize(text, font=font)
        while size > 10 and (text_size[0] > width or text_size[1] > height):
            size -= 1
            font = ImageFont.truetype(font_name, size)
            text_size = draw_proxy.textsize(text, font=font)
        return size

    def _define_font(self):
        size = self._height
        for topic in self._topics:
            size = min(
                size,
                self.get_font_size(
                    self._draw_proxy,
                    self.FONT_NAME,
                    topic,
                    self.TOPIC_WIDTH - 2 * self.CELL_OFFSET,
                    self.ROW_HEIGHT
                )
            )
        for score in self._scores:
            size = min(
                size,
                self.get_font_size(
                    self._draw_proxy,
                    self.FONT_NAME,
                    score,
                    self.SCORE_WIDTH - 2 * self.CELL_OFFSET,
                    self.ROW_HEIGHT
                )
            )
        self._font = ImageFont.truetype(
            self.FONT_NAME,
            size
        )

    def _define_coordinates(self):
        self._xs = [0, self.TOPIC_WIDTH]
        for _ in self._scores:
            self._xs.append(self._xs[-1] + self.SCORE_WIDTH)
        self._ys = [0]
        for _ in self._topics:
            self._ys.append(self._ys[-1] + self.ROW_HEIGHT)

    def _draw_grid(self):
        for x in self._xs:
            self._draw_proxy.line(
                (x, 0, x, self._height),
                fill=self.LINE_COLOR,
                width=self.LINE_WIDTH
            )
        for y in self._ys:
            self._draw_proxy.line(
                (0, y, self._width, y),
                fill=self.LINE_COLOR,
                width=self.LINE_WIDTH
            )

    def _draw_texts(self):
        for y, row in enumerate(self._rows):
            for x, col in enumerate(row):
                if not col:
                    continue
                text_size = self._draw_proxy.textsize(col, font=self._font)
                if x == 0:
                    offx = self.CELL_OFFSET
                else:
                    offx = self._xs[x] + (self._xs[x + 1] - self._xs[x] - text_size[0]) // 2
                offy = self._ys[y] + (self._ys[y + 1] - self._ys[y] - text_size[1]) // 2
                self._draw_proxy.text((offx, offy), col, font=self._font, fill=self.TEXT_COLOR)



    def draw(self, data, result):
        self._parse_data(data)

        self._define_geometry()
        self._define_image()
        self._define_font()
        self._define_coordinates()

        self._draw_grid()
        self._draw_texts()

        self._image.save(result, format='png')



def _parse_args():
    parser = argparse.ArgumentParser(description="Draw score table in png")
    parser.add_argument('data', help='json file with score table')
    parser.add_argument('output', help='where to put result image')
    return parser.parse_args()

def _main(args):
    with open(args.data, encoding='utf-8') as fin:
        data = json.load(fin)
    TableDrawer().draw(data, args.output)

if __name__ == '__main__':
    _main(_parse_args())
