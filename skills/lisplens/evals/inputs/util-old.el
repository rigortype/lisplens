;;; util.el --- helpers  -*- lexical-binding: t; -*-
(defun util-add (a b) (+ a b))
(defun util-greet (name) (message "Hi %s" name))
(defun util-legacy () (error "unused"))
(defvar util-count 0 "A counter.")
(provide 'util)
