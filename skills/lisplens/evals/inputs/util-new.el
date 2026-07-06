;;; util.el --- helpers  -*- lexical-binding: t; -*-
(defun util-add (a b)
  (+ a b))
(defun util-greet (name) (message "Hello, %s!" name))
(defun util-scale (x factor) (* x factor))
(defvar util-count 0 "A counter.")
(provide 'util)
